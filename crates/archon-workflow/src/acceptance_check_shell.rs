//! Issue 554: what the shell that runs a check says about the check's text.
//!
//! The host runs every check as a script on the stdin of
//! [`archon_shell::resolve_posix_shell`] (`sh -s`). A text that this shell
//! cannot parse never reaches its assertions, whatever its exit status:
//! under bash's POSIX mode a parse error after `trap ... EXIT` ends the
//! script with the trap's status, which is 0. So the host asks the same
//! shell to parse the text (`-n`) before it runs it
//! ([`shell_parse_error`]). That parse is the one gate: stderr cannot say
//! which process printed a syntax error (a child shell a negative test runs
//! prints the same text), so no rule reads a syntax error from a run's
//! output as the check's own.

use std::io::Write;
use std::sync::LazyLock;

use regex::Regex;

/// Builtins that change how the shell parses the lines after them: an
/// alias definition or a `shopt` option (such as `extglob`). Parse-only
/// mode runs neither, so it may reject a text that runs fine.
static PARSE_CHANGING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(?:^|[\s;&|(){}`])(?:alias|shopt)(?:\s|;|$)").expect("static pattern")
});

/// A `set -o NAME` / `set +o NAME` option.
static SET_OPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)(?:^|[\s;&|(){}`])set\s+[-+]o\s+([A-Za-z_-]+)").expect("static pattern")
});

/// `set -o` options that change what runs, never how a line parses.
const RUN_ONLY_OPTIONS: &[&str] = &[
    "allexport",
    "braceexpand",
    "errexit",
    "errtrace",
    "functrace",
    "hashall",
    "histexpand",
    "history",
    "ignoreeof",
    "monitor",
    "noclobber",
    "noexec",
    "noglob",
    "notify",
    "nounset",
    "onecmd",
    "physical",
    "pipefail",
    "privileged",
    "verbose",
    "xtrace",
];

/// Whether `command` uses a builtin that may change how its later lines
/// parse: `alias`, `shopt`, or a `set -o` option other than the ones that
/// only change what runs.
pub fn changes_parsing(command: &str) -> bool {
    PARSE_CHANGING.is_match(command)
        || SET_OPTION.captures_iter(command).any(|found| {
            found
                .get(1)
                .is_some_and(|name| !RUN_ONLY_OPTIONS.contains(&name.as_str()))
        })
}

/// The diagnostic of the executing shell's parse-only mode (`-n`) for
/// `command`, as that shell printed it; `None` when it parses the text. The
/// shell is the one every check runs under, with no environment: parsing
/// runs no command, so no search path or variable takes part. When that
/// shell cannot be started this is `None` too: the run itself then fails to
/// start, and that failure says so. A text that changes how later lines
/// parse ([`changes_parsing`]) is never judged here: its run decides.
pub fn shell_parse_error(command: &str) -> Option<String> {
    if changes_parsing(command) {
        return None;
    }
    let mut shell = archon_shell::spawn::command(archon_shell::resolve_posix_shell());
    let mut child =
        archon_shell::spawn::replace_environment(&mut shell, std::iter::empty::<(&str, &str)>())
            .args(["-n", "-s"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .ok()?;
    let mut stdin = child.stdin.take()?;
    let text = command.as_bytes().to_vec();
    // The shell stops reading at its first syntax error, so the write may
    // fail with a closed pipe: the shell's status decides, never the write.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&text);
    });
    let output = child.wait_with_output().ok()?;
    let _ = writer.join();
    if output.status.success() {
        return None;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let lines: Vec<&str> = (stderr.lines())
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    Some(if lines.is_empty() {
        format!("its parse-only mode exited with {}", output.status)
    } else {
        lines.join("\n")
    })
}

/// The author's finding for check `id`, whose text the shell that runs it
/// rejects with `error` before it runs.
pub fn parse_finding(id: &str, error: &str) -> String {
    format!(
        "check '{id}': the shell that runs checks ({}) rejects its text in parse-only mode (`-n`), so the host never ran it: no command of it would reach an assertion, and its exit status would not be its criterion's verdict. The shell reported:\n{error}\nFix exactly that shell syntax so the whole text parses; keep every assertion and every call into the deliverable, and never replace a call to the deliverable with a stand-in",
        archon_shell::resolve_posix_shell().display()
    )
}

// Every case runs a real POSIX shell.
#[cfg(all(test, unix))]
#[path = "acceptance_check_shell_tests.rs"]
mod tests;
