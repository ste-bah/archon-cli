//! Which agent-reported commands the host will run itself, and how.
//!
//! An ALLOW-list: `cargo test` or `cargo nextest run`, then only options the
//! host knows select or shape a test run -- package and target selection,
//! features, profile, parallelism, output verbosity -- and plain positional
//! filters; after `--`, only libtest's selection and display options. Every
//! word is plain (letters, digits, `_ : . , = @ -`: no shell operator, quote,
//! substitution, redirection or path separator). Anything else -- an option
//! that writes a file (`--logfile`), names a program, manifest, toolchain,
//! configuration or directory, or one the host does not know -- refuses the
//! whole command, so the host never runs it.

/// cargo / nextest options taking no value.
const FLAGS: [&str; 20] = [
    "--lib",
    "--bins",
    "--tests",
    "--doc",
    "--all-targets",
    "--workspace",
    "--all",
    "--all-features",
    "--no-default-features",
    "--release",
    "--no-fail-fast",
    "--locked",
    "--offline",
    "--frozen",
    "-q",
    "--quiet",
    "-v",
    "-vv",
    "--verbose",
    "--no-capture",
];
/// cargo / nextest options taking one plain value (`--opt v` or `--opt=v`).
const VALUED: [&str; 11] = [
    "-p",
    "--package",
    "--bin",
    "--test",
    "--exclude",
    "--features",
    "-F",
    "-j",
    "--jobs",
    "--profile",
    "--test-threads",
];
/// libtest options after `--` taking no value.
const LIBTEST_FLAGS: [&str; 9] = [
    "--exact",
    "--nocapture",
    "--no-capture",
    "--include-ignored",
    "--ignored",
    "--show-output",
    "-q",
    "--quiet",
    "--color=never",
];
/// libtest options after `--` taking one plain value.
const LIBTEST_VALUED: [&str; 2] = ["--test-threads", "--skip"];

fn plain(word: &str) -> bool {
    !word.is_empty()
        && word.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '.' | ',' | '=' | '@' | '-')
        })
}

/// Whether every word from `words` on is a known option (with its plain
/// value) or a positional filter.
fn allowed(words: &[&str], flags: &[&str], valued: &[&str]) -> bool {
    let mut at = 0;
    while at < words.len() {
        let word = words[at];
        if !word.starts_with('-') {
            at += 1;
            continue;
        }
        if flags.contains(&word) {
            at += 1;
            continue;
        }
        if let Some((option, value)) = word.split_once('=')
            && valued.contains(&option)
            && !value.is_empty()
        {
            at += 1;
            continue;
        }
        if valued.contains(&word) {
            match words.get(at + 1) {
                Some(value) if !value.starts_with('-') => at += 2,
                _ => return false,
            }
            continue;
        }
        return false;
    }
    true
}

/// Whether the host may run `command` itself; see the module doc.
pub(crate) fn host_runnable(command: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    if !words.iter().all(|word| plain(word)) {
        return false;
    }
    let start = match words.as_slice() {
        ["cargo", "test", ..] => 2,
        ["cargo", "nextest", "run", ..] => 3,
        _ => return false,
    };
    let rest = &words[start..];
    let (cargo, libtest) = match rest.iter().position(|word| *word == "--") {
        Some(at) => (&rest[..at], &rest[at + 1..]),
        None => (rest, &[][..]),
    };
    allowed(cargo, &FLAGS, &VALUED) && allowed(libtest, &LIBTEST_FLAGS, &LIBTEST_VALUED)
}

/// `command` as the host runs it: cargo's verbosity options dropped, and
/// with `--no-fail-fast` before any `--`, so one failing test binary never
/// hides the verdict of the next.
pub(crate) fn complete_run(command: &str) -> String {
    // Verbosity changes which headers cargo prints, never which tests run:
    // the host runs at the default level, where every binary has a header.
    let words: Vec<&str> = command
        .split_whitespace()
        .filter(|word| !matches!(*word, "-q" | "--quiet" | "-v" | "-vv" | "--verbose"))
        .collect();
    if words.contains(&"--no-fail-fast") {
        return words.join(" ");
    }
    let start = if words.get(1) == Some(&"nextest") {
        3
    } else {
        2
    };
    let at = words
        .iter()
        .position(|word| *word == "--")
        .unwrap_or(words.len())
        .max(start.min(words.len()));
    let mut out: Vec<&str> = words[..at].to_vec();
    out.push("--no-fail-fast");
    out.extend(&words[at..]);
    out.join(" ")
}

/// Whether the harness reported a verdict for every test binary it ran: as
/// many libtest `test result:` lines as `Running` / `Doc-tests` headers (at
/// least one), or nextest's `Summary [` line. A run stopped part-way --
/// a build failure, a kill -- is no verdict.
pub(crate) fn harness_reported(output: &str) -> bool {
    let lines: Vec<&str> = output.lines().map(str::trim_start).collect();
    if lines.iter().any(|line| line.starts_with("Summary [")) {
        return true;
    }
    let headers = lines
        .iter()
        .filter(|line| {
            (line.starts_with("Running ") && !line.starts_with("Running `"))
                || line.starts_with("Doc-tests ")
        })
        .count();
    let results = lines
        .iter()
        .filter(|line| line.starts_with("test result: "))
        .count();
    results > 0 && results == headers
}

/// The failures the harness counted: the `N failed` of every libtest
/// `test result:` line summed, or nextest's `Summary` count. `None` when a
/// summary line carries no count the host can read.
pub(crate) fn failed_count(output: &str) -> Option<usize> {
    let summaries: Vec<&str> = output
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("test result: ") || line.starts_with("Summary ["))
        .collect();
    if summaries.is_empty() {
        return None;
    }
    let mut total = 0usize;
    for line in summaries {
        let words: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == ';' || c == ',')
            .filter(|w| !w.is_empty())
            .collect();
        let count = words
            .windows(2)
            .find(|pair| pair[1] == "failed")
            .and_then(|pair| pair[0].parse::<usize>().ok());
        match count {
            Some(count) => total += count,
            // A summary with no failure count (all passed on nextest).
            None if !line.contains("failed") => {}
            None => return None,
        }
    }
    Some(total)
}
