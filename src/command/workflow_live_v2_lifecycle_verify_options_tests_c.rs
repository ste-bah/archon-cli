/// Without a declared calendar the check must stay silent -- it is opt-in, and
/// a 24/7 venue trades every day.
use super::*;

#[test]
fn a_venue_with_no_declared_calendar_keeps_every_session() {
    let rows: Vec<_> = (0..40)
        .map(|index| {
            let jitter = ((index * 7919) % 977) as f64 / 100.0;
            serde_json::json!({"timestamp": 1_704_067_200i64 + index * 86_400,
                               "value": 100.0 + index as f64 * 0.4 + jitter,
                               "measure": 50.0 + index as f64 * 0.9 + jitter})
        })
        .collect();
    let (project, mut contract) = series_project(rows);
    contract["observed_time_field"] = serde_json::json!("timestamp");
    let stdout = verifier_stdout(&project, &contract);
    assert!(!stdout.contains("when the venue was closed"), "{stdout}");
}

/// A markdown deliverable declared as `text` must pass on existence rather
/// than being parsed as JSON. Without this the verifier demoted correct work
/// permanently: no remediation can make prose parse, so the task looped to its
/// round cap and blocked on a defect that was in the contract, not the work.
#[test]
fn a_textual_deliverable_is_checked_for_presence_not_parsed_as_json() {
    let project = tempfile::tempdir().expect("project");
    let artifact = project.path().join(".archon/demo/inventory.md");
    std::fs::create_dir_all(artifact.parent().expect("parent")).expect("dir");
    std::fs::write(&artifact, "# Inventory\n\nRules are cited.\n").expect("artifact");
    let contract = serde_json::json!({
        "kind": "rule_inventory",
        "artifact_path": ".archon/demo/inventory.md",
        "artifact_format": "text"
    });
    let command = super::super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(
        stdout.contains("declared_text_deliverable_present"),
        "{stdout}"
    );
}

/// Still fail-closed: declaring `text` buys presence, not a free pass.
#[test]
fn a_missing_textual_deliverable_still_fails() {
    let project = tempfile::tempdir().expect("project");
    let contract = serde_json::json!({
        "kind": "rule_inventory",
        "artifact_path": ".archon/demo/absent.md",
        "artifact_format": "text"
    });
    let command = super::super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("missing or empty"),
        "status={:?}
stdout:
{}
stderr:
{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Inference must not weaken JSON validation: a `.json` deliverable with no
/// declared format is still strictly parsed and still fails when malformed.
#[test]
fn an_undeclared_json_extension_is_still_strictly_parsed() {
    let project = tempfile::tempdir().expect("project");
    let artifact = project.path().join(".archon/demo/thing.json");
    std::fs::create_dir_all(artifact.parent().expect("parent")).expect("dir");
    std::fs::write(&artifact, "# not json\n").expect("artifact");
    let contract = serde_json::json!({
        "kind": "thing",
        "artifact_path": ".archon/demo/thing.json"
    });
    let command = super::super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("not valid JSON"),
        "status={:?}
stdout:
{}
stderr:
{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Site 2: the PARAMETERIZED/instance path. A markdown deliverable declared
/// with a `<placeholder>` segment goes down the instance branch, which loaded
/// every instance as JSON independently of the single-artifact branch. Fixing
/// only the single-artifact site left this one broken — and it is the one a
/// per-run report artifact actually travels through, so a helper-level test
/// would have reported the fix working while it still failed live.
#[test]
fn a_parameterized_markdown_instance_is_not_parsed_as_json() {
    let project = tempfile::tempdir().expect("project");
    let report = project.path().join(".archon/demo/runs/run-1/review.md");
    std::fs::create_dir_all(report.parent().expect("parent")).expect("dir");
    std::fs::write(&report, "# Adversarial review\n\nNo blocking issues.\n").expect("report");
    write_json(
        &project.path().join(".archon/demo/runs.json"),
        &serde_json::json!({"records": {"run-1": {"report_path": ".archon/demo/runs/run-1/review.md"}}}),
    );
    let contract = serde_json::json!({
        "kind": "run_review",
        "artifact_path": ".archon/demo/runs/<run-id>/review.md",
        "instance_source_path": ".archon/demo/runs.json",
        "instance_source_records_field": "records",
        "instance_artifact_field": "report_path"
    });
    let command = super::super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "markdown instance must not be JSON-parsed: {stdout}"
    );
    assert!(!stdout.contains("not valid JSON"), "{stdout}");
}

/// Still fail-closed on the instance path: a declared instance that is missing
/// fails regardless of its extension.
#[test]
fn a_missing_parameterized_instance_still_fails() {
    let project = tempfile::tempdir().expect("project");
    write_json(
        &project.path().join(".archon/demo/runs.json"),
        &serde_json::json!({"records": {"run-1": {"report_path": ".archon/demo/runs/run-1/review.md"}}}),
    );
    let contract = serde_json::json!({
        "kind": "run_review",
        "artifact_path": ".archon/demo/runs/<run-id>/review.md",
        "instance_source_path": ".archon/demo/runs.json",
        "instance_source_records_field": "records",
        "instance_artifact_field": "report_path"
    });
    let command = super::super::workflow_live_v2_deliverable_contract::verification_command(
        project.path().to_str().expect("project path"),
        &contract,
    );
    let out = run_verifier(&command);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("missing or empty"),
        "status={:?}
stdout:
{}
stderr:
{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
