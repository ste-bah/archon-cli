//! Issue-39: landing tools refuse hollow records (blank skeleton entries, a verify pass with no succeeded command), normalise `commands_run` like the envelope, and name both counts on a records_landed mismatch.
use super::record_landing::{enum_names, schema_hint, RecordKind, RecordLanding, VERIFY_VERDICTS};
use serde_json::{json, Value};

fn open(kind: RecordKind, subjects: &[&str]) -> (tempfile::TempDir, RecordLanding) {
    let temp = tempfile::tempdir().unwrap();
    let landing = RecordLanding::open(temp.path().join("records"), "identity".into(), kind, subjects.iter().map(|s| s.to_string()).collect(), kind != RecordKind::Skeleton).unwrap();
    (temp, landing)
}
fn evidence() -> Value { json!([{"kind":"test","summary":"ran the suite"}]) }
fn skeleton_task(implements: Value, contracts: Value) -> Value {
    json!({"subject":"TASK-CORE-002","summary":"one task","task":{"task_id":"TASK-CORE-002","file_name":"TASK-CORE-002.md","implements":implements,"deliverable_contracts":contracts}})
}

#[test]
fn skeleton_blank_obligation_id_is_rejected_by_path() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let error = landing.land(skeleton_task(json!([" "]), json!([]))).unwrap_err().to_string();
    assert!(error.contains("task.implements[0]"), "{error}");
    assert!(error.contains("Expected schema:"), "{error}");
    let error = landing.land(skeleton_task(json!(["REQ-1",""]), json!([]))).unwrap_err().to_string();
    assert!(error.contains("task.implements[1]"), "{error}");
    landing.land(skeleton_task(json!(["REQ-1"]), json!([]))).unwrap();
}

#[test]
fn skeleton_contract_with_blank_kind_or_path_is_rejected_by_path() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let error = landing.land(skeleton_task(json!([]), json!([{"kind":"","artifact_path":"x"}]))).unwrap_err().to_string();
    assert!(error.contains("task.deliverable_contracts[0].kind"), "{error}");
    let error = landing.land(skeleton_task(json!([]), json!([{"kind":"report","artifact_path":"out/a.json"},{"kind":"report","artifact_path":"  "}]))).unwrap_err().to_string();
    assert!(error.contains("task.deliverable_contracts[1].artifact_path"), "{error}");
    assert!(error.contains("Expected schema:"), "{error}");
    assert!(landing.assemble(&json!({"records_landed":0})).unwrap()["tasks"].as_array().unwrap().is_empty(), "nothing hollow was retained");
}

#[test]
fn verify_pass_without_a_succeeded_command_is_rejected() {
    let (_temp, landing) = open(RecordKind::Verify, &["S"]);
    let expected = "verify record with status accepted/noop requires at least one succeeded commands_run entry with a captured output_summary";
    for status in ["accepted", "noop"] {
        let error = landing.land(json!({"subject":"S","status":status,"summary":"passed","evidence":evidence(),"commands_run":[]})).unwrap_err().to_string();
        assert!(error.contains(expected), "{status}: {error}");
        assert!(error.contains("Expected schema:"), "{error}");
    }
    let skipped = json!([{"kind":"test","command":"run-suite","status":"skipped","output_summary":"not run"}]);
    let error = landing.land(json!({"subject":"S","status":"accepted","summary":"passed","evidence":evidence(),"commands_run":skipped})).unwrap_err().to_string();
    assert!(error.contains(expected), "a skipped entry is not a pass: {error}");
    let blank = json!([{"kind":"test","command":"run-suite","status":"succeeded","exit_code":0,"output_summary":"  "}]);
    let error = landing.land(json!({"subject":"S","status":"accepted","summary":"passed","evidence":evidence(),"commands_run":blank})).unwrap_err().to_string();
    assert!(error.contains(expected), "a blank output_summary is synthesised, not captured: {error}");
    let no_command = json!([{"kind":"test","command":" ","status":"succeeded","exit_code":0,"output_summary":"12 passed"}]);
    let error = landing.land(json!({"subject":"S","status":"accepted","summary":"passed","evidence":evidence(),"commands_run":no_command})).unwrap_err().to_string();
    assert!(error.contains(expected), "a blank command is not a command: {error}");
    assert_eq!(landing.remaining().unwrap(), vec!["S"], "nothing landed");
}

#[test]
fn verify_pass_with_a_succeeded_command_lands_and_a_failure_needs_none() {
    let (_temp, landing) = open(RecordKind::Verify, &["S"]);
    landing.land(json!({"subject":"S","status":"accepted","summary":"passed","evidence":evidence(),"commands_run":[{"kind":"test","command":"run-suite","status":"succeeded","exit_code":0,"output_summary":"12 passed"}]})).unwrap();
    let record = &landing.assemble(&json!({"records_landed":1})).unwrap()["verification_records"][0];
    assert_eq!(record["status"], "accepted");
    assert_eq!(record["commands_run"][0]["output_summary"], "12 passed");
    landing.land(json!({"subject":"S","status":"failed","summary":"could not build","evidence":evidence(),"replace":true})).unwrap();
    assert_eq!(landing.assemble(&json!({"records_landed":1})).unwrap()["verification_records"][0]["status"], "failed");
}

#[test]
fn commands_run_is_normalised_like_the_envelope_before_deserialising() {
    let (_temp, landing) = open(RecordKind::Review, &["S"]);
    landing.land(json!({"subject":"S","summary":"looked","findings":[],"evidence":evidence(),"commands_run":[{"command":"run-suite","exit_code":0}]})).unwrap();
    let command = &landing.assemble(&json!({"records_landed":1})).unwrap()["verification_records"][0]["commands_run"][0];
    assert_eq!(command["kind"], "other");
    assert_eq!(command["status"], "succeeded");
    assert_eq!(command["command"], "run-suite");
    let summary = command["output_summary"].as_str().unwrap();
    assert!(summary.starts_with(crate::v2::agent_output_normalize::SYNTHESIZED_OUTPUT_SUMMARY_PREFIX), "{summary}");
    let error = landing.land(json!({"subject":"S","evidence":evidence(),"commands_run":[{"command":"run-suite"}]})).unwrap_err().to_string();
    assert!(error.contains("invalid record at commands_run[0].status: missing field `status`"), "no exit_code, nothing to derive from: {error}");
    let error = landing.land(json!({"subject":"S","evidence":[{"summary":"x"}]})).unwrap_err().to_string();
    assert!(error.contains("invalid record at evidence[0].kind: missing field `kind`"), "evidence stays strict: {error}");
}

#[test]
fn synthesised_output_summary_does_not_satisfy_a_verify_pass() {
    let (_temp, landing) = open(RecordKind::Verify, &["S"]);
    let error = landing.land(json!({"subject":"S","status":"accepted","summary":"passed","evidence":evidence(),"commands_run":[{"kind":"test","command":"run-suite","exit_code":0}]})).unwrap_err().to_string();
    assert!(error.contains("requires at least one succeeded commands_run entry with a captured output_summary"), "{error}");
}

#[test]
fn records_landed_mismatch_names_both_counts() {
    let (_temp, landing) = open(RecordKind::Review, &["S"]);
    landing.land(json!({"subject":"S","summary":"looked","evidence":evidence()})).unwrap();
    let error = landing.assemble(&json!({"records_landed":3})).unwrap_err().to_string();
    assert!(error.contains("records_landed=3 but host retained 1 record(s)"), "{error}");
    let error = landing.assemble(&json!({})).unwrap_err().to_string();
    assert!(error.contains("records_landed=missing but host retained 1 record(s)"), "{error}");
}

#[test]
fn enum_names_are_the_lists_the_schema_hint_prints() {
    let names = enum_names();
    let hint = schema_hint();
    for list in [&names.evidence_kinds, &names.command_kinds, &names.command_statuses, &names.statuses] {
        assert!(!list.is_empty());
        assert!(hint.contains(&list.join("|")), "{hint}");
    }
    assert_eq!(names.verify_verdicts, vec!["accepted", "noop", "failed", "blocked", "needs_review"]);
    assert_eq!(names.verify_verdicts.len(), VERIFY_VERDICTS.len());
    assert!(names.verify_verdicts.iter().all(|v| names.statuses.contains(v)));
}
