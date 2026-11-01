//! Issue 554: what the shell that runs a check says about the check's text.
//!
//! The host runs every check as a script on the stdin of
//! [`archon_shell::resolve_posix_shell`] (`sh -s`). A text that this shell
//! cannot parse never reaches its assertions, whatever its exit status:
//! under bash's POSIX mode a parse error after `trap ... EXIT` ends the
//! script with the trap's status, which is 0. So the host asks the same
//! shell to parse the text (`-n`) before it runs it
//! ([`shell_parse_error`]), and a run whose own shell reported a syntax
//! error is never a pass ([`exit_zero_syntax_defect`]).

use std::io::Write;
use std::sync::LazyLock;

use regex::Regex;

use crate::acceptance_check_crash::ScriptDefect;
use crate::acceptance_scratch::CheckResult;

/// The shell's own syntax-error line for a script it reads from stdin
/// (`sh: line N:`, dash's `sh: N:`), or for text that script evaluated
/// (`sh: eval: line N:`, dash's `sh: N: eval:`). A nested `sh -c` names
/// `-c` and a script run by path names that path: neither matches.
static OWN_SYNTAX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:/[^\s:]*/)?(?:sh|bash|dash|ksh): (?:eval: )?(?:line )?\d+: (?:eval: )?(?:syntax error|Syntax error)",
    )
    .expect("static pattern")
});

/// A syntax error bash reports for a file by its path (`path: line N:`).
static FILE_SYNTAX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([^\s:][^:]*): line \d+: syntax error").expect("static pattern")
});

/// The diagnostic of the executing shell's parse-only mode (`-n`) for
/// `command`, as that shell printed it; `None` when it parses the text. The
/// shell is the one every check runs under, with no environment: parsing
/// runs no command, so no search path or variable takes part. When that
/// shell cannot be started this is `None` too: the run itself then fails to
/// start, and that failure says so.
pub fn shell_parse_error(command: &str) -> Option<String> {
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

/// The first line of `stderr` that is a syntax error of the check's own
/// shell, or of a file the check text `command` sources by that path
/// (`. path`, `source path`).
pub fn own_syntax_error<'a>(command: &str, stderr: &'a str) -> Option<&'a str> {
    stderr.lines().map(str::trim).find(|line| {
        OWN_SYNTAX.is_match(line)
            || FILE_SYNTAX
                .captures(line)
                .and_then(|found| found.get(1))
                .is_some_and(|path| sources(command, path.as_str()))
    })
}

/// Whether `command` sources `path` by that exact spelling.
fn sources(command: &str, path: &str) -> bool {
    Regex::new(&format!(
        r#"(?m)(?:^|[\s;&|(])(?:\.|source)\s+['"]?{}['"]?(?:\s|;|&|\||\)|$)"#,
        regex::escape(path)
    ))
    .is_ok_and(|pattern| pattern.is_match(command))
}

/// Issue 554: a run that exited 0 although its own shell reported a syntax
/// error: its exit status is not its criterion's verdict, so it is the
/// check's own defect, never a pass. `lines` are its stderr's non-empty
/// lines.
pub(crate) fn exit_zero_syntax_defect(command: &str, lines: &[&str]) -> Option<ScriptDefect> {
    let signal = own_syntax_error(command, &lines.join("\n"))?.to_string();
    let rule = if shell_parse_error(command).is_some() {
        "syntax error in the check's shell script, after which it still exited 0"
    } else {
        "a shell syntax error in text the check ran or sourced, after which it still exited 0"
    };
    Some(ScriptDefect {
        interpreter: "shell".into(),
        rule: rule.into(),
        signal,
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

impl CheckResult {
    /// Issue 554: whether this run exited 0 although its own shell reported
    /// a syntax error, so its exit status is no pass: the capture's own
    /// classification of the raw streams when it has one, else the streams
    /// a verdict reads.
    pub fn reports_shell_syntax_error(&self) -> bool {
        if self.exit_code != Some(0) {
            return false;
        }
        match &self.classification {
            Some(classification) => matches!(
                classification.crash,
                crate::acceptance_check_crash::CheckRunClass::ScriptDefect(_)
            ),
            None => own_syntax_error("", &String::from_utf8_lossy(self.verdict_stderr())).is_some(),
        }
    }
}

#[cfg(test)]
#[path = "acceptance_check_shell_tests.rs"]
mod tests;
