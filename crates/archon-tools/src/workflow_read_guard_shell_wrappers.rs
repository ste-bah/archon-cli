//! Reading a simple command past the process wrappers that only bound,
//! time or set the environment of what they run, and matching a run of a
//! declared command token for token (Issue-120 and its follow-up).
use super::{commands, redirect_operator};

/// Issue-120: `words` past the process wrappers that only bound, time or
/// set the environment of the command they run (`timeout 900 cargo test`,
/// `nice -n 5 make`, `time -f %e go test`, `env RUST_LOG=x cargo test`,
/// `PATH=/opt/bin timeout 60 make`), so the program underneath is the one
/// judged. Leading `NAME=value` assignments, each wrapper's own options with
/// their operands, and `timeout`'s duration are skipped; the slice returned
/// starts at the program word. A wrapper whose options cannot be followed
/// statically (`env -S`, `command -v`), or one with nothing after it, leaves
/// `words` as they are.
pub(super) fn unwrapped(words: &[String]) -> &[String] {
    let mut at = 0;
    loop {
        while words.get(at).is_some_and(|word| assignment(word)) {
            at += 1;
        }
        let Some(head) = words.get(at) else {
            return words;
        };
        let name = head.rsplit('/').next().unwrap_or(head);
        let rest = &words[at + 1..];
        let Some(skip) = wrapper_operands(name, rest) else {
            return if at == 0 { words } else { &words[at..] };
        };
        if skip >= rest.len() {
            return words;
        }
        at += 1 + skip;
    }
}

/// A shell variable assignment word: `NAME=value`, NAME a shell identifier.
fn assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// How many words after wrapper `name` are its own options and operands, or
/// `None` when `name` is not a transparent wrapper (or is used in a form that
/// does not simply run the rest, or cannot be followed statically).
fn wrapper_operands(name: &str, rest: &[String]) -> Option<usize> {
    // Options that take the NEXT word as their value, per wrapper.
    let valued: &[&str] = match name {
        "timeout" => &["-s", "-k", "--signal", "--kill-after"],
        "nice" => &["-n", "--adjustment"],
        "time" => &["-f", "-o", "--format", "--output"],
        "env" => &["-u", "-C", "--unset", "--chdir"],
        "nohup" | "command" => &[],
        _ => return None,
    };
    let mut i = 0;
    while let Some(word) = rest.get(i) {
        if word == "--" {
            i += 1;
            break;
        }
        if !word.starts_with('-') || word == "-" {
            break;
        }
        let bare = word.split_once('=').map_or(word.as_str(), |(flag, _)| flag);
        match (name, bare) {
            // `env -S` splits its operand into a command line; `command -v`
            // looks a name up instead of running it.
            ("env", "-S" | "--split-string") | ("command", "-v" | "-V") => return None,
            _ => {}
        }
        i += if valued.contains(&word.as_str()) {
            2
        } else {
            1
        };
    }
    match name {
        "timeout" => Some(i + 1),
        _ => Some(i),
    }
}

/// Issue-120 follow-up: whether `command` runs `declared` itself, token for
/// token, not merely text containing it. Both are split into simple commands
/// (on `;`, `&&`, `||`, `|`, newlines), each read past its process wrappers
/// ([`unwrapped`]) with redirections and their operands dropped; `declared`'s
/// simple commands must appear, in order and contiguous, among `command`'s.
/// So `cd /repo && timeout 900 cargo test -p a 2>&1 | tail` runs `cargo test
/// -p a`; `cargo test -p ab`, `cargo test -p a --lib` and `echo cargo test
/// -p a` do not.
pub(in crate::workflow_read_guard) fn runs_declared(command: &str, declared: &str) -> bool {
    let want = invocations(declared);
    !want.is_empty()
        && invocations(command)
            .windows(want.len())
            .any(|window| window == want.as_slice())
}

fn invocations(text: &str) -> Vec<Vec<String>> {
    commands(text)
        .iter()
        .map(|words| {
            let mut out = Vec::new();
            let mut operand = false;
            for word in unwrapped(words) {
                if std::mem::take(&mut operand) {
                    continue;
                }
                if redirect_operator(word) {
                    operand = true;
                    continue;
                }
                out.push(word.clone());
            }
            out
        })
        .filter(|words| !words.is_empty())
        .collect()
}

/// A path-scoped restore of named files to the baseline commit, as one
/// simple command's words: `git checkout [HEAD] -- <path>...` or `git
/// restore [--worktree|-W] [--source=HEAD|-s HEAD] [--] <path>...`. It
/// rewrites those files to what the landing already has, touching no
/// history, branch or index, so the git-mutation refusal admits it in an
/// isolated item worktree: an agent asked to undo a build side effect in
/// another holder's file has no other way to (a Write or a redirect to that
/// path is refused as a write there).
///
/// Refused, so it stays a refused mutation: anything before `git` (an
/// assignment such as `GIT_DIR=...`, `env`, a wrapper, a path to the
/// binary), any git global option (`-C`, `-c`, `--git-dir`, `--work-tree`,
/// ...), any other form (a branch or commit, `--staged`, other options),
/// and any path that is absolute, climbs (`..`), is shell-expanded (`$`, a
/// backtick, `~`, a glob or brace), is pathspec magic (`:`), or names the
/// whole tree (`.`, `./`, `a/..`). The caller admits it only as the whole
/// command (one simple command, no subshell, group or substitution). Not an
/// inspection: `inspection` still reads
/// it as a write.
pub(super) fn baseline_file_restore(words: &[String]) -> bool {
    let mut words = words.iter().map(String::as_str).peekable();
    if words.next() != Some("git") {
        return false;
    }
    match words.next() {
        Some("checkout") => {
            if words.peek() == Some(&"HEAD") {
                words.next();
            }
            if words.next() != Some("--") {
                return false;
            }
        }
        Some("restore") => {
            while let Some(word) = words.peek().copied() {
                match word {
                    "--worktree" | "-W" | "--source=HEAD" => {
                        words.next();
                    }
                    "-s" | "--source" => {
                        words.next();
                        if words.next() != Some("HEAD") {
                            return false;
                        }
                    }
                    "--" => {
                        words.next();
                        break;
                    }
                    _ if word.starts_with('-') => return false,
                    _ => break,
                }
            }
        }
        _ => return false,
    }
    let paths: Vec<&str> = words.collect();
    !paths.is_empty() && paths.iter().all(|path| plain_worktree_path(path))
}

/// A literal path inside the working directory that names one entry below
/// it: relative, no `..`, nothing a shell expands, not pathspec magic, and
/// not the directory itself once `.` segments are dropped.
fn plain_worktree_path(path: &str) -> bool {
    if path.starts_with(['/', '-', ':', '~'])
        || path.contains(['$', '`', '*', '?', '[', ']', '{', '}', '(', ')', '\\'])
    {
        return false;
    }
    let mut segments = 0;
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => return false,
            _ => segments += 1,
        }
    }
    segments > 0
}
