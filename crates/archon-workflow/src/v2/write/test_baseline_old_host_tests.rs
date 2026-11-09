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
