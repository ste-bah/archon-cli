use super::*;
use archon_workflow::task_set_contract::AcceptanceCriterion;
use std::collections::BTreeMap;

fn criterion() -> AcceptanceCriterion {
    serde_json::from_value(serde_json::json!({
        "id":"AC-EVID-418", "criterion":"command fails before implementation",
        "check":{"kind":"command","command":"printf volatile","cwd":"project_root"},
        "judgment":{"verdict":"accepted","counterexample":"","reason":"","host_call_id":""}
    }))
    .unwrap()
}

#[cfg(unix)]
fn run(command: &str) -> archon_workflow::acceptance_scratch::CheckResult {
    let output = std::process::Command::new("/bin/sh")
        .args(["-c", command])
        .output()
        .unwrap();
    archon_workflow::acceptance_scratch::CheckResult {
        fault: None,
        acceptance_id: "AC-EVID-418".into(),
        exit_code: output.status.code(),
        quota_walk_count: 0,
        stdout: output.stdout,
        stderr: output.stderr,
        environment_note: None,
        operational_error: None,
        classification: None,
    }
}

fn key(
    client: &EvidenceJudge,
    entry: &AcceptanceCriterion,
    shown_output: &str,
    input_key: Option<&str>,
    result: &archon_workflow::acceptance_scratch::CheckResult,
) -> String {
    let shown = serde_json::json!({
        "id": entry.id,
        "criterion": entry.criterion,
        "check": entry.check,
        "baseline": {"commit":"fixture-tree", "stdout": shown_output}
    });
    super::super::passability::key(
        client,
        "test-model",
        &shown,
        entry,
        &BTreeMap::new(),
        input_key,
        result,
    )
}

fn evidence(case: &str, before: impl std::fmt::Display, after: impl std::fmt::Display) {
    println!("EVIDENCE #418 {case} BEFORE: {before} AFTER: {after}");
}

// Expected before running: real executions that differ only in PID and
// temporary filename produce one stable passability key for the same inputs
// and exit status.
#[test]
#[cfg(unix)]
fn evidence_418_volatile_pid_and_temp_output_reuses_key() {
    let client = EvidenceJudge::default();
    let entry = criterion();
    let first = run("printf 'pid=%s temp=%s\\n' \"$$\" \"$(mktemp)\"; exit 1");
    let second = run("printf 'pid=%s temp=%s\\n' \"$$\" \"$(mktemp)\"; exit 1");
    let first_output = String::from_utf8(first.stdout.clone()).unwrap();
    let second_output = String::from_utf8(second.stdout.clone()).unwrap();
    let first_key = key(
        &client,
        &entry,
        &first_output,
        Some("input-closure-A"),
        &first,
    );
    let second_key = key(
        &client,
        &entry,
        &second_output,
        Some("input-closure-A"),
        &second,
    );
    assert_ne!(
        first_output, second_output,
        "the real process output varies"
    );
    assert_eq!(first.exit_code, Some(1));
    assert_eq!(second.exit_code, Some(1));
    assert_eq!(first_key, second_key);
    let separate_read = String::from_utf8(second.stdout).unwrap();
    evidence(
        "volatile-output-same-inputs-and-exit",
        format!(
            "stdout={first_output:?}; exit={:?}; key={first_key}",
            first.exit_code
        ),
        format!(
            "separate captured stdout={separate_read:?}; exit={:?}; key={second_key}",
            second.exit_code
        ),
    );
}

// Expected before running: changing the check text or probe input closure
// changes the key even when command output is volatile.
#[test]
#[cfg(unix)]
fn evidence_418_changed_check_or_input_changes_key() {
    let client = EvidenceJudge::default();
    let entry = criterion();
    let first = run("printf 'pid=%s\\n' \"$$\"; exit 1");
    let output = String::from_utf8(first.stdout.clone()).unwrap();
    let base = key(&client, &entry, &output, Some("input-closure-A"), &first);
    let mut changed = entry.clone();
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, .. } =
        &mut changed.check
    {
        command.push_str("; printf changed-check");
    }
    let changed_check = key(&client, &changed, &output, Some("input-closure-A"), &first);
    let changed_input = key(&client, &entry, &output, Some("input-closure-B"), &first);
    assert_ne!(base, changed_check);
    assert_ne!(base, changed_input);
    evidence(
        "changed-check-and-probe-input",
        format!("check={:?}; probe=input-closure-A; key={base}", entry.check),
        format!(
            "check={:?}; probe=input-closure-B; check-key={changed_check}; input-key={changed_input}",
            changed.check
        ),
    );
}

// Expected before running: a changed real command exit status changes the
// key while retaining the same check and probe input closure.
#[test]
#[cfg(unix)]
fn evidence_418_changed_exit_status_changes_key() {
    let client = EvidenceJudge::default();
    let entry = criterion();
    let failed = run("printf 'pid=%s\\n' \"$$\"; exit 1");
    let passed = run("printf 'pid=%s\\n' \"$$\"; exit 0");
    let failed_output = String::from_utf8(failed.stdout.clone()).unwrap();
    let passed_output = String::from_utf8(passed.stdout.clone()).unwrap();
    let failed_key = key(
        &client,
        &entry,
        &failed_output,
        Some("input-closure-A"),
        &failed,
    );
    let passed_key = key(
        &client,
        &entry,
        &passed_output,
        Some("input-closure-A"),
        &passed,
    );
    assert_eq!(failed.exit_code, Some(1));
    assert_eq!(passed.exit_code, Some(0));
    assert_ne!(failed_key, passed_key);
    evidence(
        "changed-exit-status",
        format!(
            "output={failed_output:?}; exit={:?}; key={failed_key}",
            failed.exit_code
        ),
        format!(
            "output={passed_output:?}; exit={:?}; key={passed_key}",
            passed.exit_code
        ),
    );
}
