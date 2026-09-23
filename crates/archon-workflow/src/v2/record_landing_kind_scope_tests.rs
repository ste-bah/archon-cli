//! The record contract is scoped to the landing kind: a kind is shown only the fields its own path reads, a field it ignores cannot fail its landing, and a field it reads is accepted string-encoded as well as inline.
use super::record_landing::{RecordKind, RecordLanding, schema_hint};
use serde_json::{Value, json};

fn open(kind: RecordKind, subjects: &[&str]) -> (tempfile::TempDir, RecordLanding) {
    let temp = tempfile::tempdir().unwrap();
    let landing = RecordLanding::open(
        temp.path().join("records"),
        "identity".into(),
        kind,
        subjects.iter().map(|s| s.to_string()).collect(),
        kind != RecordKind::Skeleton,
    )
    .unwrap();
    (temp, landing)
}
fn verify_record(task: Value) -> Value {
    json!({"subject":"TASK-CORE-002","summary":"checked the deliverable","status":"failed",
        "evidence":[{"kind":"test","summary":"ran the focused check"}],
        "findings":[{"claim":"the artifact is absent","task_id":"TASK-CORE-002"}],
        "task":task})
}
fn skeleton_task() -> Value {
    json!({"task_id":"TASK-CORE-002","file_name":"TASK-CORE-002.md","blocks":["TASK-CORE-003"],
        "implements":["REQ-4"],
        "deliverable_contracts":[{"kind":"report","artifact_path":"out/report.json","min_instances":1}]})
}

/// The live rejection: a verify record carried the skeleton-only field as a
/// JSON string, the strict parse ran before the kind was considered, and the
/// whole record was refused over a field the verify path never reads.
#[test]
fn verify_record_carrying_a_string_encoded_ignored_field_lands() {
    let (_temp, landing) = open(RecordKind::Verify, &["TASK-CORE-002"]);
    landing
        .land(verify_record(json!(
            "{\"blocks\": [\"TASK-CORE-003\"], \"deliverable_contracts\": []}"
        )))
        .unwrap();
    let data = landing.assemble(&json!({"records_landed":1})).unwrap();
    assert_eq!(data["verification_records"][0]["status"], json!("failed"));
    assert!(
        data["tasks"].as_array().unwrap().is_empty(),
        "an ignored field is dropped, never retained: {}",
        data["tasks"]
    );
}

#[test]
fn verify_record_carrying_a_well_formed_ignored_field_lands() {
    let (_temp, landing) = open(RecordKind::Verify, &["TASK-CORE-002"]);
    landing.land(verify_record(skeleton_task())).unwrap();
    let data = landing.assemble(&json!({"records_landed":1})).unwrap();
    assert_eq!(data["verification_records"][0]["status"], json!("failed"));
    assert!(
        data["tasks"].as_array().unwrap().is_empty(),
        "{}",
        data["tasks"]
    );
}

#[test]
fn review_record_carrying_an_ignored_field_lands() {
    let (_temp, landing) = open(RecordKind::Review, &["TASK-CORE-002"]);
    landing
        .land(json!({"subject":"TASK-CORE-002","summary":"read it",
            "evidence":[{"kind":"review","summary":"read the implementation"}],
            "task":"{\"implements\": []}"}))
        .unwrap();
    assert_eq!(
        landing.assemble(&json!({"records_landed":1})).unwrap()["verification_records"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

/// Dropping the ignored field must not loosen anything else: an unknown field
/// is still refused, and a malformed field the kind does read still fails.
#[test]
fn every_other_field_stays_exactly_as_strict() {
    let (_temp, landing) = open(RecordKind::Verify, &["TASK-CORE-002"]);
    let mut record = verify_record(json!("{}"));
    record["not_a_record_field"] = json!("x");
    let error = landing.land(record).unwrap_err().to_string();
    assert!(
        error.contains("unknown field `not_a_record_field`"),
        "{error}"
    );
    let mut record = verify_record(json!("{}"));
    record["evidence"] = json!("not an array of evidence");
    let error = landing.land(record).unwrap_err().to_string();
    assert!(error.contains("invalid record at evidence"), "{error}");
}

#[test]
fn skeleton_record_with_a_string_encoded_task_lands() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    landing
        .land(json!({"subject":"TASK-CORE-002","summary":"one task",
            "task":serde_json::to_string(&skeleton_task()).unwrap()}))
        .unwrap();
    let landed = &landing.assemble(&json!({"records_landed":1})).unwrap()["tasks"][0];
    assert_eq!(landed["task_id"], json!("TASK-CORE-002"));
    assert_eq!(landed["implements"], json!(["REQ-4"]));
    assert_eq!(landed["deliverable_contracts"][0]["kind"], json!("report"));
}

#[test]
fn skeleton_string_that_is_not_json_still_names_the_field_and_the_schema() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let error = landing
        .land(json!({"subject":"TASK-CORE-002","summary":"one task","task":"not json at all"}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("record field `task`"), "{error}");
    assert!(error.contains("not valid JSON"), "{error}");
    assert!(error.contains("Expected schema:"), "{error}");
}

/// Issue-38/Issue-39 must survive the decode: a string-encoded stub is the
/// same stub, and an absent entry is still an absent entry.
#[test]
fn skeleton_hollow_or_absent_task_is_still_rejected() {
    let (_temp, landing) = open(RecordKind::Skeleton, &[]);
    let error = landing
        .land(json!({"subject":"TASK-CORE-002","summary":"one task"}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("skeleton record requires task"), "{error}");
    let stub = "{\"task_id\":\"TASK-CORE-002\",\"file_name\":\"TASK-CORE-002.md\"}";
    let error = landing
        .land(json!({"subject":"TASK-CORE-002","summary":"one task","task":stub}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("cannot be owned or verified"), "{error}");
    let blank =
        "{\"task_id\":\"TASK-CORE-002\",\"file_name\":\"TASK-CORE-002.md\",\"implements\":[\" \"]}";
    let error = landing
        .land(json!({"subject":"TASK-CORE-002","summary":"one task","task":blank}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("task.implements[0]"), "{error}");
    assert!(
        landing.assemble(&json!({"records_landed":0})).unwrap()["tasks"]
            .as_array()
            .unwrap()
            .is_empty(),
        "nothing hollow was retained"
    );
}

#[test]
fn only_the_skeleton_contract_shows_the_skeleton_entry() {
    let skeleton = schema_hint(RecordKind::Skeleton);
    assert!(
        skeleton.contains("task (skeleton records only): {task_id:"),
        "{skeleton}"
    );
    assert!(skeleton.contains("deliverable_contracts"), "{skeleton}");
    for kind in [RecordKind::Verify, RecordKind::Review] {
        let hint = schema_hint(kind);
        assert!(!hint.contains("task (skeleton records only)"), "{hint}");
        assert!(!hint.contains("deliverable_contracts"), "{hint}");
        assert!(!hint.contains("file_name: <task_id>.md"), "{hint}");
        assert!(hint.contains("summary: string, replace?: bool}"), "{hint}");
        assert!(hint.starts_with("{subject: string"), "{hint}");
        assert!(!hint.contains('\n'), "the hint is one line: {hint}");
    }
}

/// The hint the agent is handed carries its own kind's contract, not a shared one.
#[test]
fn the_landing_hint_carries_the_contract_for_its_own_kind() {
    for kind in [RecordKind::Verify, RecordKind::Review, RecordKind::Skeleton] {
        let (_temp, landing) = open(kind, &["TASK-CORE-002"]);
        let hint = landing.hint().unwrap();
        assert!(
            hint.contains(&format!("Schema: {}", schema_hint(kind))),
            "{hint}"
        );
        assert_eq!(
            hint.contains("task (skeleton records only)"),
            kind == RecordKind::Skeleton,
            "{hint}"
        );
        assert_eq!(
            hint.contains(",task}"),
            kind == RecordKind::Skeleton,
            "{hint}"
        );
    }
}
