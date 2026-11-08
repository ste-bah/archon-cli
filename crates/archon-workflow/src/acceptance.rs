//! Phase B implementation-stage acceptance binding.
//!
//! An `implementation` stage is accepted ONLY when both conditions hold:
//!   1. every declared `expected_target_files` entry exists after the stage, AND
//!   2. the stage `verify_command` (when present) exits with status 0.
//!
//! This is the structural guard that makes a write-capable stage trustworthy:
//! a stage that leaves declared targets missing — or whose verification fails —
//! is rejected rather than silently accepted. Existing unchanged targets are
//! allowed so resumed/idempotent workflows can report already-satisfied work
//! without being forced to touch files pointlessly.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[cfg(test)]
#[path = "acceptance_unavailable_tests.rs"]
mod unavailable_environment_tests;

/// A fingerprint of a single target path. `None` means the path is absent.
pub type TargetFingerprints = BTreeMap<String, Option<String>>;

/// Outcome of evaluating implementation-stage acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptanceOutcome {
    Accepted,
    Rejected(String),
    /// Verification could not be launched or its operator environment was unavailable.
    Unavailable(String),
}

/// Captured result for a focused stage verification command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifyCommandReport {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub environment_note: Option<String>,
    /// Issue 554: the error of the executing shell's parse-only mode when it
    /// rejected the command, which then never ran.
    pub parse_error: Option<String>,
}

impl AcceptanceOutcome {
    pub fn is_accepted(&self) -> bool {
        matches!(self, AcceptanceOutcome::Accepted)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            AcceptanceOutcome::Accepted => None,
            AcceptanceOutcome::Rejected(reason) | AcceptanceOutcome::Unavailable(reason) => {
                Some(reason.as_str())
            }
        }
    }
}

#[path = "acceptance_verify_report.rs"]
mod verify_report;

/// Fingerprint each declared target relative to `root` (or absolute).
pub fn snapshot_targets(root: &Path, targets: &[String]) -> TargetFingerprints {
    targets
        .iter()
        .map(|target| (target.clone(), fingerprint(root, target)))
        .collect()
}

/// Targets that still do not exist after execution.
pub fn missing_targets(after: &TargetFingerprints) -> Vec<String> {
    after
        .iter()
        .filter(|(_, fingerprint)| fingerprint.is_none())
        .map(|(target, _)| target.clone())
        .collect()
}

/// Targets whose fingerprints changed during execution.
pub fn mutated_targets(before: &TargetFingerprints, after: &TargetFingerprints) -> Vec<String> {
    after
        .keys()
        .filter(|target| before.get(*target) != after.get(*target))
        .cloned()
        .collect()
}

/// Run the stage verification command in `root`. Returns `Ok(())` on exit 0,
/// otherwise an error describing the failure. `None` command always passes.
pub fn run_verify_command(root: &Path, command: Option<&str>) -> Result<(), String> {
    run_verify_command_with_policy(root, command, None)
}

/// Execute with a policy supplied by the operator's configuration boundary.
pub fn run_verify_command_with_policy(
    root: &Path,
    command: Option<&str>,
    policy: Option<&crate::acceptance_check_environment::CheckPolicy>,
) -> Result<(), String> {
    let Some(report) = run_verify_command_capture(root, command, policy)? else {
        return Ok(());
    };
    if report.success() {
        return Ok(());
    }
    Err(report.failure_reason())
}

/// Run the stage verification command in `root` and return captured output.
/// `None` and empty commands return `Ok(None)`.
pub(crate) fn run_verify_command_capture(
    root: &Path,
    command: Option<&str>,
    policy: Option<&crate::acceptance_check_environment::CheckPolicy>,
) -> Result<Option<VerifyCommandReport>, String> {
    run_verify_with_command(root, command, policy, |environment| {
        environment.command_with_report(shell_program())
    })
}

fn run_verify_with_command(
    root: &Path,
    command: Option<&str>,
    policy: Option<&crate::acceptance_check_environment::CheckPolicy>,
    prepare: impl Fn(
        &crate::acceptance_check_environment::CommandEnvironment,
    ) -> (
        std::process::Command,
        archon_shell::read_boundary::LaunchReport,
    ),
) -> Result<Option<VerifyCommandReport>, String> {
    let Some(command) = command else {
        return Ok(None);
    };
    let command = command.trim();
    if command.is_empty() {
        return Ok(None);
    }
    let _host_run = archon_shell::spawn::host_run();
    let environment = crate::acceptance_check_environment::CommandEnvironment::capture(policy)?
        .with_remedy("Supply an operator-owned CheckPolicy (with the needed names in forwarded) to run_verify_command_with_policy or evaluate_with_policy; those APIs consume the explicit policy, not a project config file");
    // Issue 554: the shell that runs it parses it first (`-n`); under bash's
    // POSIX mode a parse error after `trap ... EXIT` exits with the trap's 0.
    // The rule is unconditional (`acceptance_check_shell::PARSE_RULE`), and
    // it reads only the command, never a file it sources.
    let (mut process, boundary_report) = prepare(&environment);
    process.arg("-n").arg("-c").arg(command).current_dir(root);
    archon_shell::read_boundary::prepare_spawn(&process)
        .map_err(|err| format!("verify_command failed to launch: {err}; {boundary_report}"))?;
    let parsed = process.output().map_err(|err| {
        format!("verify_command parse check failed to launch: {err}; {boundary_report}")
    })?;
    if !parsed.status.success() {
        let stderr = String::from_utf8_lossy(&parsed.stderr).trim().to_string();
        return Ok(Some(VerifyCommandReport {
            command: command.to_string(),
            exit_code: parsed.status.code(),
            stdout: String::new(),
            parse_error: Some(if stderr.is_empty() {
                format!("its parse-only mode exited with {}", parsed.status)
            } else {
                stderr.clone()
            }),
            stderr,
            environment_note: archon_shell::read_boundary::observation_note(boundary_report),
        }));
    }
    let (mut process, boundary_report) = prepare(&environment);
    process.arg("-c").arg(command).current_dir(root);
    let output = archon_shell::read_boundary::prepare_spawn(&process)
        .and_then(|()| process.output())
        .map_err(|err| format!("verify_command failed to launch: {err}; {boundary_report}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let environment_note =
        verify_environment_note(&environment, &output, boundary_report.to_string());
    Ok(Some(VerifyCommandReport {
        command: command.to_string(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
        environment_note,
        parse_error: None,
    }))
}

pub(crate) fn verify_environment_note(
    environment: &crate::acceptance_check_environment::CommandEnvironment,
    output: &std::process::Output,
    boundary_report: String,
) -> Option<String> {
    let mut note = archon_shell::read_boundary::observation_note(boundary_report);
    if !output.status.success()
        && let Some(reason) = environment.note(&[&output.stdout, &output.stderr])
    {
        note = Some(match note {
            Some(boundary) => format!("{reason} {boundary}"),
            None => reason,
        });
    }
    note
}

/// The shell used to run `verify_command`.
///
/// Plain `sh` everywhere it resolves. On Windows it usually does not: Git for
/// Windows ships `sh.exe`, but its installer only adds `<git>\cmd` to PATH,
/// which holds `git.exe` and not `sh.exe`. So a machine with Git properly
/// installed still failed to launch any `verify_command`, and the feature was
/// simply unavailable there.
///
/// Rather than require PATH surgery, locate `sh.exe` next to the `git.exe`
/// that is already on PATH — `<git>\cmd\git.exe` puts it at `<git>\bin\sh.exe`.
/// Falls back to bare `sh` so the error message stays the familiar one when no
/// shell can be found at all.
#[cfg(windows)]
pub(crate) fn shell_program() -> std::ffi::OsString {
    use std::path::PathBuf;

    if archon_shell::spawn::command("sh")
        .arg("-c")
        .arg("exit 0")
        .output()
        .is_ok()
    {
        return "sh".into();
    }
    let Ok(output) = archon_shell::spawn::command("where").arg("git").output() else {
        return "sh".into();
    };
    let Some(first) = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(PathBuf::from)
    else {
        return "sh".into();
    };
    // <git>\cmd\git.exe -> <git>\bin\sh.exe
    let candidate = first
        .parent()
        .and_then(|cmd_dir| cmd_dir.parent())
        .map(|git_root| git_root.join("bin").join("sh.exe"));
    match candidate {
        Some(path) if path.is_file() => path.into_os_string(),
        _ => "sh".into(),
    }
}

#[cfg(not(windows))]
pub(crate) fn shell_program() -> std::ffi::OsString {
    "sh".into()
}

/// Combine the mutation check and verification into a single acceptance verdict.
pub fn evaluate(
    root: &Path,
    targets: &[String],
    _before: &TargetFingerprints,
    after: &TargetFingerprints,
    verify_command: Option<&str>,
) -> AcceptanceOutcome {
    evaluate_with_policy(root, targets, _before, after, verify_command, None)
}

/// Stage acceptance with explicit operator check policy.
pub fn evaluate_with_policy(
    root: &Path,
    targets: &[String],
    _before: &TargetFingerprints,
    after: &TargetFingerprints,
    verify_command: Option<&str>,
    policy: Option<&crate::acceptance_check_environment::CheckPolicy>,
) -> AcceptanceOutcome {
    if targets.is_empty() {
        return AcceptanceOutcome::Rejected(
            "implementation stage declared no expected_target_files".to_string(),
        );
    }
    let missing = missing_targets(after);
    if !missing.is_empty() {
        return AcceptanceOutcome::Rejected(format!(
            "expected_target_files missing after implementation: {}",
            missing.join(", ")
        ));
    }
    match run_verify_command_capture(root, verify_command, policy) {
        Err(reason) => AcceptanceOutcome::Unavailable(reason),
        Ok(Some(report)) if !report.success() => {
            AcceptanceOutcome::Rejected(report.failure_reason())
        }
        Ok(_) => AcceptanceOutcome::Accepted,
    }
}

fn fingerprint(root: &Path, target: &str) -> Option<String> {
    let path = resolve(root, target);
    let bytes = std::fs::read(path).ok()?;
    Some(blake3::hash(&bytes).to_hex().to_string())
}

fn resolve(root: &Path, target: &str) -> PathBuf {
    let raw = Path::new(target);
    if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        root.join(raw)
    }
}

fn command_is_discovery_only(command: &str) -> bool {
    let command = command.to_ascii_lowercase();
    [
        "--list",
        " --list-tests",
        " list-tests",
        " test list",
        "--collect-only",
        " collect-only",
        " gradle tasks",
        "go test -list",
        "dotnet test --list-tests",
    ]
    .iter()
    .any(|needle| command.contains(needle))
}

/// Whether a test command's output says it ran no test at all (A3).
///
/// Counted, not matched: a runner that prints one summary per test binary
/// (`running 0 tests` for every binary a filter left empty) did work when
/// any binary ran a test, so every `running N tests` count is read and the
/// output is zero-work only when all of them are zero. The other runners'
/// phrases match only as whole numbers: `0 examples`, never `10 examples`,
/// and `0 passed; 0 failed`, never `10 passed; 0 failed`.
pub fn output_reports_zero_work(stdout: &str, stderr: &str) -> bool {
    let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    let counts = test_work_counts(&text);
    if !counts.is_empty() {
        return counts.iter().all(|count| *count == 0);
    }
    [
        "0 passed; 0 failed",
        "0 examples",
        "0 checks",
        "no tests collected",
        "collected 0 items",
        "no matching tests",
        "no tests ran",
        "ran 0 tests",
        "0 tests run",
        "0 tests completed",
        "test result: ok. 0 passed",
    ]
    .iter()
    .any(|needle| whole_number_match(&text, needle))
}

/// `needle` occurs in `text` and, when it starts with a digit, not as the
/// tail of a longer number.
fn whole_number_match(text: &str, needle: &str) -> bool {
    let numeric = needle.starts_with(|c: char| c.is_ascii_digit());
    text.match_indices(needle).any(|(at, _)| {
        !numeric
            || text[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_fingerprint_helpers_detect_missing_and_mutated() {
        let mut before = TargetFingerprints::new();
        before.insert("a".into(), Some("h1".into()));
        before.insert("b".into(), None);
        before.insert("c".into(), Some("h3".into()));
        let mut after = TargetFingerprints::new();
        after.insert("a".into(), Some("h1".into())); // unchanged
        after.insert("b".into(), Some("h2".into())); // created
        after.insert("c".into(), None); // missing after execution
        let mutated = mutated_targets(&before, &after);
        let missing = missing_targets(&after);
        assert_eq!(mutated, vec!["b".to_string(), "c".to_string()]);
        assert_eq!(missing, vec!["c".to_string()]);
    }

    #[test]
    fn verify_command_none_and_empty_pass() {
        let root = std::env::temp_dir();
        assert!(run_verify_command(&root, None).is_ok());
        assert!(run_verify_command(&root, Some("   ")).is_ok());
    }

    // These drive POSIX shell semantics (`exit 3`, `printf`, `;`). They run on
    // Windows too now that `shell_program()` finds the `sh.exe` Git ships.
    #[test]
    fn verify_command_failure_is_reported() {
        let root = std::env::temp_dir();
        let err = run_verify_command(&root, Some("exit 3")).unwrap_err();
        assert!(err.contains('3'), "reason should carry exit code: {err}");
    }

    #[test]
    fn verify_command_list_only_is_not_completion_evidence() {
        let root = std::env::temp_dir();
        let err = run_verify_command(&root, Some("printf listed; true --list")).unwrap_err();
        assert!(err.contains("discovery/list-only"), "{err}");
    }

    #[test]
    fn verify_command_zero_work_output_is_not_completion_evidence() {
        let root = std::env::temp_dir();
        let err = run_verify_command(&root, Some("printf 'running 0 tests\\n'")).unwrap_err();
        assert!(err.contains("zero-test/no-op"), "{err}");
    }

    #[test]
    fn zero_work_is_counted_not_matched() {
        // One binary ran the filtered test; the others ran none.
        let mixed = "running 0 tests\ntest result: ok. 0 passed; 0 failed\nrunning 1 test\ntest it ... ok\ntest result: ok. 1 passed; 0 failed\n";
        assert!(!output_reports_zero_work(mixed, ""));
        assert!(output_reports_zero_work(
            "running 0 tests\nrunning 0 tests\n",
            ""
        ));
        // A larger count is never read as zero.
        assert!(!output_reports_zero_work(
            "test result: ok. 10 passed; 0 failed",
            ""
        ));
        assert!(!output_reports_zero_work("10 examples, 0 failures", ""));
        assert!(output_reports_zero_work("0 examples, 0 failures", ""));
        assert!(output_reports_zero_work("", "collected 0 items"));
        assert!(!output_reports_zero_work("all 12 checks passed", ""));
    }

    #[test]
    fn evaluate_rejects_when_no_targets_declared() {
        let root = std::env::temp_dir();
        let before = TargetFingerprints::new();
        let after = TargetFingerprints::new();
        let outcome = evaluate(&root, &[], &before, &after, None);
        assert!(!outcome.is_accepted());
    }

    #[test]
    fn evaluate_accepts_existing_unchanged_targets_for_idempotent_work() {
        let root = std::env::temp_dir();
        let mut before = TargetFingerprints::new();
        before.insert("Cargo.toml".into(), Some("h1".into()));
        let mut after = TargetFingerprints::new();
        after.insert("Cargo.toml".into(), Some("h1".into()));
        let outcome = evaluate(&root, &["Cargo.toml".into()], &before, &after, None);
        assert!(outcome.is_accepted());
    }

    #[test]
    fn evaluate_rejects_targets_still_missing_after_execution() {
        let root = std::env::temp_dir();
        let mut before = TargetFingerprints::new();
        before.insert("src/new.rs".into(), None);
        let after = before.clone();
        let outcome = evaluate(&root, &["src/new.rs".into()], &before, &after, None);
        assert_eq!(
            outcome.reason(),
            Some("expected_target_files missing after implementation: src/new.rs")
        );
    }
}

/// Runner blocks, in capture order, for host inventory diagnostics. A mixed
/// capture is not globally zero-work; each invocation still needs its inventory.
pub fn test_work_counts(text: &str) -> Vec<u64> {
    static RUNNING: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RUNNING
        .get_or_init(|| regex::Regex::new(r"\brunning (\d+) tests?\b").expect("static regex"))
        .captures_iter(text)
        .filter_map(|caps| caps.get(1)?.as_str().parse().ok())
        .collect()
}

#[cfg(test)]
#[path = "acceptance_report_tests.rs"]
mod report_tests;
