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
#[path = "acceptance_check_shell_lex.rs"]
mod lex;

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

/// Whether `command` runs a command that may change how its later lines
/// parse: `alias`, `shopt`, or a `set -o` option other than the ones that
/// only change what runs, in any quoted or escaped spelling, also through
/// `command` or `builtin`. A mention in a comment, a quoted string, a
/// here-document body or a command substitution runs none, so it is no
/// reason to lift the preflight: the gate fails closed.
pub fn changes_parsing(command: &str) -> bool {
    lex::simple_commands(command).iter().any(|words| {
        let words: Vec<&str> = (words.iter().map(String::as_str))
            .skip_while(|word| matches!(*word, "command" | "builtin"))
            .collect();
        match words.first() {
            Some(&("alias" | "shopt")) => true,
            Some(&"set") => set_changes_parsing(&words[1..]),
            _ => false,
        }
    })
}

/// Whether `set` arguments name an `-o`/`+o` option that may change parsing.
fn set_changes_parsing(arguments: &[&str]) -> bool {
    let mut arguments = arguments.iter();
    while let Some(flags) = arguments.next() {
        if (flags.starts_with('-') || flags.starts_with('+')) && flags[1..].contains('o') {
            match arguments.next() {
                Some(name) if RUN_ONLY_OPTIONS.contains(name) => {}
                Some(_) => return true,
                None => {}
            }
        }
    }
    false
}

/// The files `command` sources by a literal relative path (`. path`,
/// `source path`), resolved under `cwd`; a path built at run time is left
/// to the run.
fn sourced_files(command: &str, cwd: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    (lex::simple_commands(command).into_iter())
        .filter(|words| matches!(words.first().map(String::as_str), Some("." | "source")))
        .filter_map(|words| words.into_iter().nth(1))
        .filter(|path| {
            !path.is_empty()
                && !path.contains(['$', '*', '?', '[', '~'])
                && !std::path::Path::new(path).is_absolute()
        })
        .map(|path| {
            let resolved = cwd.join(&path);
            (path, resolved)
        })
        .filter(|(_, resolved)| resolved.is_file())
        .collect()
}

/// Whether the preflight is lifted for `command` run under `cwd`: the text,
/// or a file it sources by a literal relative path, changes how later
/// lines parse ([`changes_parsing`]).
pub fn parse_lifted(command: &str, cwd: &std::path::Path) -> bool {
    changes_parsing(command)
        || (sourced_texts(command, cwd).iter()).any(|(_, text)| changes_parsing(text))
}

fn sourced_texts(command: &str, cwd: &std::path::Path) -> Vec<(String, String)> {
    (sourced_files(command, cwd).into_iter())
        .filter_map(|(path, resolved)| Some((path, std::fs::read_to_string(resolved).ok()?)))
        .collect()
}

/// The parse-only error of `shell` (by default the checks' shell) for a
/// file `command` sources by a literal relative path under `cwd`, naming
/// that file; a file that itself changes how later lines parse is left to
/// the run.
pub fn sourced_file_error(
    command: &str,
    cwd: &std::path::Path,
    shell: Option<&std::ffi::OsStr>,
) -> Option<String> {
    let shell = shell.unwrap_or(archon_shell::resolve_posix_shell().as_os_str());
    sourced_texts(command, cwd).iter().find_map(|(path, text)| {
        let error = (!changes_parsing(text)).then(|| parse_with(shell, text))??;
        Some(format!(
            "the file `{path}` it sources does not parse: {error}"
        ))
    })
}

/// Issue 554: the preflight error for check text `command` run under `cwd`
/// by the checks' shell: the shell's error for the text itself, else for a
/// file it sources by a literal relative path; `None` when they parse or
/// the preflight is lifted ([`parse_lifted`]) for the text.
pub fn sourced_parse_error(
    command: &str,
    cwd: &std::path::Path,
    shell: Option<&std::ffi::OsStr>,
) -> Option<String> {
    let checks = archon_shell::resolve_posix_shell().as_os_str();
    if !parse_lifted(command, cwd)
        && let Some(error) = parse_with(shell.unwrap_or(checks), command)
    {
        return Some(error);
    }
    sourced_file_error(command, cwd, shell)
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
