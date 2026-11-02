//! Issue 554: what the shell that runs a check says about the check's text.
//!
//! The host runs every check as a script on the stdin of
//! [`archon_shell::resolve_posix_shell`] (`sh -s`). A text that this shell
//! cannot parse never reaches its assertions, whatever its exit status:
//! under bash's POSIX mode a parse error after `trap ... EXIT` ends the
//! script with the trap's status, which is 0. So the host asks the same
//! shell to parse the text (`-n`) before it runs it
//! ([`shell_parse_error`]).
//!
//! The check contract rule: a check (and a stage `verify_command`) must
//! parse under the executing shell's `-n`. That parse is unconditional and
//! the one syntax gate. Parse-only mode runs no command, so syntax built
//! from an alias the text defines is rejected: such a block must be
//! written out. Stderr cannot say which process printed a syntax error (a
//! child shell a negative test runs prints the same text), so no rule
//! reads one from a run's output as the check's own; and the parse reads
//! only the text, never a file it sources, which its run alone judges.

use std::io::Write;
/// The diagnostic of the executing shell's parse-only mode (`-n`) for
/// `command`, as that shell printed it; `None` when it parses the text. The
/// shell is the one every check runs under, with no environment: parsing
/// runs no command, so no search path or variable takes part. When that
/// shell cannot be started this is `None` too: the run itself then fails to
/// start, and that failure says so. Only the text is read: no file it
/// sources, which its run alone judges.
pub fn shell_parse_error(command: &str) -> Option<String> {
    parse_with(archon_shell::resolve_posix_shell().as_os_str(), command)
}

/// `shell`'s parse-only diagnostic for `command` read on its stdin.
fn parse_with(shell: &std::ffi::OsStr, command: &str) -> Option<String> {
    let mut shell = archon_shell::spawn::command(shell);
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

/// The check contract rule a rejected text breaks, stated for its author.
pub const PARSE_RULE: &str = "a check must parse under the parse-only mode (`-n`) of the shell that runs it; `-n` runs no command, so it does not apply an alias the text defines: syntax built from an alias must be written out";

/// The author's finding for check `id`, whose text the shell that runs it
/// rejects with `error` before it runs.
pub fn parse_finding(id: &str, error: &str) -> String {
    format!(
        "check '{id}': the shell that runs checks ({}) rejects its text in parse-only mode (`-n`), so the host never ran it: no command of it would reach an assertion, and its exit status would not be its criterion's verdict. The rule: {PARSE_RULE}. The shell reported:\n{error}\nFix exactly that shell syntax so the whole text parses; keep every assertion and every call into the deliverable, and never replace a call to the deliverable with a stand-in",
        archon_shell::resolve_posix_shell().display()
    )
}

// Every case runs a real POSIX shell.
#[cfg(all(test, unix))]
#[path = "acceptance_check_shell_tests.rs"]
mod tests;
