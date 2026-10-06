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

/// A fingerprint of a single target path. `None` means the path is absent.
pub type TargetFingerprints = BTreeMap<String, Option<String>>;

/// Outcome of evaluating implementation-stage acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptanceOutcome {
    Accepted,
    Rejected(String),
}

/// Captured result for a focused stage verification command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifyCommandReport {
    pub command: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl AcceptanceOutcome {
    pub fn is_accepted(&self) -> bool {
        matches!(self, AcceptanceOutcome::Accepted)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            AcceptanceOutcome::Accepted => None,
            AcceptanceOutcome::Rejected(reason) => Some(reason.as_str()),
        }
    }
}

impl VerifyCommandReport {
    pub(crate) fn success(&self) -> bool {
        self.exit_code == Some(0)
            && !command_is_discovery_only(&self.command)
            && !output_reports_zero_work(&self.stdout, &self.stderr)
    }

    pub(crate) fn failure_reason(&self) -> String {
        if self.exit_code == Some(0) && command_is_discovery_only(&self.command) {
            return "verify_command is discovery/list-only and cannot prove completion".into();
        }
        if self.exit_code == Some(0) && output_reports_zero_work(&self.stdout, &self.stderr) {
            return "verify_command produced zero-test/no-op output and cannot prove completion"
                .into();
        }
        let code = self
            .exit_code
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_string());
        format!("verify_command exited with status {code}")
    }
}

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
    let Some(command) = command else {
        return Ok(None);
    };
    let command = command.trim();
    if command.is_empty() {
        return Ok(None);
    }
    let environment = crate::acceptance_check_environment::CommandEnvironment::capture(policy)?
        .with_remedy("Supply an operator-owned CheckPolicy (with the needed names in forwarded) to run_verify_command_with_policy or evaluate_with_policy; those APIs consume the explicit policy, not a project config file");
    let output = environment
        .command(shell_program())
        .arg("-c")
        .arg(command)
        .current_dir(root)
        .output()
        .map_err(|err| format!("verify_command failed to launch: {err}"))?;
    if !output.status.success()
        && let Some(reason) = environment.failure(&[&output.stdout, &output.stderr])
    {
        return Err(reason);
    }
    let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success()
        && let Some(note) = environment.note(&[&output.stdout, &output.stderr])
    {
        stderr.push_str(&format!("\n{note}"));
    }
    Ok(Some(VerifyCommandReport {
        command: command.to_string(),
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
    }))
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
    match run_verify_command_with_policy(root, verify_command, policy) {
        Ok(()) => AcceptanceOutcome::Accepted,
        Err(reason) => AcceptanceOutcome::Rejected(reason),
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
    use std::sync::OnceLock;
    static RUNNING: OnceLock<regex::Regex> = OnceLock::new();
    let text = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    let running = RUNNING
        .get_or_init(|| regex::Regex::new(r"\brunning (\d+) tests?\b").expect("static regex"));
    let counts: Vec<u64> = running
        .captures_iter(&text)
        .filter_map(|caps| caps.get(1)?.as_str().parse().ok())
        .collect();
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
