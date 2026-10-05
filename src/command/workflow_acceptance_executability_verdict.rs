//! Did a check that failed on a tree give a VERDICT there (Issue 328)?
//!
//! A can-fail proof (A4) is a check failing on the tree before any
//! implementation. That failure proves something only when the check itself
//! decided it: its assertion found the criterion false, or the deliverable
//! it drives is absent there. A run that stopped before the check could
//! decide anything -- a program it runs could not start, the tree did not
//! build, its own script crashed, it never exited -- gave NO verdict: it
//! would fail the same way whatever the check asserts, so it proves nothing
//! about the check. That run is unproven (the host's, resumable), never a
//! proof.
//!
//! Test harnesses draw the same line, mostly through their exit status:
//! pytest exits 1 when tests failed and 2-5 when they were interrupted,
//! could not be collected, hit an internal or usage error, or found none;
//! POSIX `grep`, `diff` and `cmp` exit 1 for a negative answer and 2 for
//! trouble; a POSIX shell exits 126 or 127 when a program cannot be run or
//! is not found. Other runners share one status for both: `cargo test` exits
//! 101 for a failed test and for a compile error, jest 1 for a failed test
//! and for a suite that failed to run; only their output separates them (a
//! test result summary versus a compiler error). A check is any script, so
//! the host reads no runner's status as its own; it applies the conventions
//! every one of these keeps:
//!
//! 1. exit 126 or 127: the shell could not start a program it looked up by
//!    NAME on the search path, or by an absolute path, or a script's
//!    interpreter (a tool or interpreter the environment lacks). A program
//!    named by a RELATIVE path (`./bin/tool`, `bash scripts/new.sh`) is a
//!    file of the tree: its absence is the deliverable's absence, a verdict.
//!    An exit 126/127 whose missing program the shell does not name is
//!    taken as no verdict;
//! 2. no exit status: the run was killed before it reported anything;
//! 3. the check crashed in its own code
//!    ([`archon_workflow::acceptance_check_crash`]);
//! 4. the output carries a compiler error at a source location, in a shape
//!    compilers share -- `path:line[:col]: [fatal ]error...` (the GNU
//!    convention), `path(line,col): error...`, an `error...:` header whose
//!    primary span is `--> path:line[:col]`, or an interpreter's
//!    `SyntaxError` in a source file -- and nothing shows an assertion ran:
//!    no runner reports a failed test (`N failed`, `FAILED`), no panic, no
//!    failed assertion. The tree did not build, so nothing was asserted.
//!
//! Anything else is the check's verdict on that tree: any other non-zero
//! exit, and a run that did no work (no test matched): the deliverable it
//! needs is absent there, which is exactly what a pre-implementation tree
//! must show. A failure that names no source location (a test target or
//! subcommand that does not exist yet) is the same. No tool, language or
//! PRD is named here.

use std::sync::LazyLock;

use archon_workflow::acceptance_check_crash::{CheckRunClass, classify_check_run};
use archon_workflow::acceptance_scratch::CheckResult;
use regex::Regex;

/// Why the failed run `result` of the check text `command` gave no verdict
/// (see the module docs); `None` when it passed, could not run at all (an
/// operational error, reported as such), or its failure is a verdict.
pub(crate) fn no_verdict(command: &str, result: &CheckResult) -> Option<String> {
    if let Some(why) = host_failure(result) {
        return Some(why);
    }
    if result.operational_error.is_some() || super::baseline::passed(result) {
        return None;
    }
    match classify_check_run(command, result) {
        CheckRunClass::ScriptDefect(defect) => Some(format!(
            "it crashed in its own {} code ({}) before it asserted anything",
            defect.interpreter, defect.rule
        )),
        CheckRunClass::Passed | CheckRunClass::Failed => None,
    }
}

/// Why `result` failed for its host rather than for its check -- rules 1, 2
/// and 4 of the module docs -- or `None`. Such a run is never remembered as
/// a verdict: once the host can run the check, it runs again.
pub(crate) fn host_failure(result: &CheckResult) -> Option<String> {
    if result.operational_error.is_some() || super::baseline::passed(result) {
        return None;
    }
    let Some(code) = result.exit_code else {
        return Some(
            "it ended without an exit status (killed by a signal) before it reported anything"
                .to_string(),
        );
    };
    if matches!(code, 126 | 127) {
        let stderr = String::from_utf8_lossy(&result.stderr);
        // A file of the tree that is not there yet: the deliverable's absence.
        if missing_program(&stderr).is_some_and(|name| tree_file(&name)) {
            return None;
        }
        return Some(format!(
            "a program it runs could not be started (exit {code}: not found, or not executable), a tool or interpreter its environment lacks"
        ));
    }
    if code == 0 {
        // It ran and did no work: the deliverable is absent, a verdict.
        return None;
    }
    let output = format!(
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    (!ASSERTED.is_match(&output) && compile_error(&output)).then(|| {
        "the tree it ran on did not build: it stopped at a compiler error at a source location before any assertion ran".to_string()
    })
}

/// The shell's report of a program it could not start: `sh: 1: NAME: not
/// found`, `bash: line 1: NAME: command not found`, `bash: NAME: No such
/// file or directory`, `env: 'NAME': No such file or directory`.
static NOT_STARTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^[^:\s]+:(?: line \d+:| \d+:)? (.+?): (?:command not found|not found|No such file or directory|Permission denied)\s*$",
    )
    .expect("static pattern")
});

/// The program the last such report in `stderr` names; `None` when there is
/// none, or it is a script whose interpreter is missing.
fn missing_program(stderr: &str) -> Option<String> {
    let line = (stderr.lines().rev()).find(|line| NOT_STARTED.is_match(line.trim_end()))?;
    if line.contains("bad interpreter") {
        return None;
    }
    let name = NOT_STARTED.captures(line.trim_end())?.get(1)?.as_str();
    Some(
        name.trim_matches(|c| matches!(c, '\'' | '"' | '`'))
            .to_string(),
    )
}

/// Whether `name` is a path relative to the check's working directory: a
/// file of the tree, not a program looked up on the search path.
fn tree_file(name: &str) -> bool {
    name.contains('/') && !name.starts_with('/') && !name.contains(char::is_whitespace)
}

/// Evidence that an assertion ran: a runner's count of failed tests, a
/// failed-test marker, a panic, or a failed assertion.
static ASSERTED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b[1-9][0-9]* (?:failed|failing)\b|\bFAILED\b|panicked at|\bAssertionError\b|[Aa]ssertion\b[^\n]*\bfailed",
    )
    .expect("static pattern")
});

/// `path:line[:col]: [fatal ]error...` and `path(line,col): error...`.
static LOCATED_ERROR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\S+?(?::\d+(?::\d+)?:|\(\d+,\d+\):?)\s+(?:fatal\s+)?error\b")
        .expect("static pattern")
});

/// An error diagnostic's header: `error: ...`, `error[E0425]: ...`.
static ERROR_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:fatal\s+)?error(?:\[[A-Za-z0-9]+\])?:\s").expect("static pattern")
});

/// A diagnostic's primary span: `--> path:line[:col]`.
static SPAN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-->\s+\S+:\d+").expect("static pattern"));

/// A source file's interpreter frame: `File "path", line N`, not a program
/// read from stdin or `-c`.
static SOURCE_FRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^\s*File "([^"<][^"]*)", line \d+"#).expect("static pattern"));

const SYNTAX_ERRORS: [&str; 3] = ["SyntaxError", "IndentationError", "TabError"];

/// Whether `output` carries a compiler error at a source location (see the
/// module docs).
fn compile_error(output: &str) -> bool {
    let mut header = false;
    for line in output.lines() {
        if line.trim().is_empty() {
            header = false;
            continue;
        }
        if LOCATED_ERROR.is_match(line) || (header && SPAN.is_match(line.trim_start())) {
            return true;
        }
        // Any other unindented line (a warning's header, plain output) ends
        // the error's block: a span after it is not the error's.
        if !line.starts_with(char::is_whitespace) {
            header = ERROR_HEADER.is_match(line);
        }
    }
    let lines: Vec<&str> = output.lines().filter(|l| !l.trim().is_empty()).collect();
    let Some(last) = lines.last().map(|line| line.trim()) else {
        return false;
    };
    let syntax = SYNTAX_ERRORS.iter().any(|class| {
        last.strip_prefix(class)
            .is_some_and(|rest| rest.starts_with(':'))
    });
    syntax && lines.iter().any(|line| SOURCE_FRAME.is_match(line))
}

#[cfg(test)]
#[path = "workflow_acceptance_executability_verdict_tests.rs"]
mod tests;
