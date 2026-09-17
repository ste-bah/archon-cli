//! Issue-35/36: landing rejections name the offending path and the full schema; `replace:true` supersedes an earlier landing.
use super::record_landing::{schema_hint, RecordKind, RecordLanding};
use serde_json::json;

fn review(subjects: &[&str]) -> (tempfile::TempDir, RecordLanding) {
    let temp = tempfile::tempdir().unwrap();
    let landing = RecordLanding::open(temp.path().join("records"), "identity".into(), RecordKind::Review, subjects.iter().map(|s| s.to_string()).collect()).unwrap();
    (temp, landing)
}

#[test]
fn missing_nested_field_names_its_path_and_the_schema() {
    let (_temp, landing) = review(&["S"]);
    let error = landing.land(json!({"subject":"S","evidence":[{"summary":"x"}]})).unwrap_err().to_string();
    assert!(error.contains("invalid record at evidence[0].kind: missing field `kind`"), "{error}");
    assert!(error.contains("Expected schema: {subject: string"), "{error}");
    assert!(error.contains("evidence: [{kind: inspection|implementation|test|review|remediation|blocker|artifact|other, summary: string, source?: string}]+"), "{error}");
    assert!(error.contains("commands_run: [{kind: inspect|test|build|format|review|other, command: string, status: succeeded|failed|skipped"), "{error}");
    assert!(error.contains("status?: pending|running|accepted|noop|failed|blocked|needs_review|cancelled"), "{error}");
}

#[test]
fn deeper_missing_field_and_top_level_missing_subject_are_both_located() {
    let (_temp, landing) = review(&["S"]);
    let error = landing.land(json!({"subject":"S","evidence":[{"kind":"test","summary":"ran"}],"commands_run":[{"kind":"test","command":"run","status":"failed"}]})).unwrap_err().to_string();
    assert!(error.contains("invalid record at commands_run[0].output_summary: missing field `output_summary`"), "{error}");
    let error = landing.land(json!({"evidence":[{"kind":"test","summary":"ran"}]})).unwrap_err().to_string();
    assert!(error.contains("invalid record at subject: missing field `subject`"), "{error}");
    assert!(error.contains("Expected schema:"), "{error}");
}

#[test]
fn unknown_top_level_key_is_named_with_the_schema() {
    let (_temp, landing) = review(&["S"]);
    let error = landing.land(json!({"subject":"S","kind":"review","evidence":[{"kind":"test","summary":"ran"}]})).unwrap_err().to_string();
    assert!(error.contains("invalid record at kind: unknown field `kind`"), "{error}");
    assert!(error.contains("Expected schema:"), "{error}");
    assert!(error.contains("replace?: bool"), "{error}");
}

#[test]
fn validation_rejections_keep_their_message_and_append_the_schema() {
    let (_temp, landing) = review(&["S"]);
    let error = landing.land(json!({"subject":"other","evidence":[{"kind":"test","summary":"ran"}]})).unwrap_err().to_string();
    assert!(error.contains("record subject is outside this call. Expected schema: {subject"), "{error}");
    let error = landing.land(json!({"subject":"S"})).unwrap_err().to_string();
    assert!(error.contains("record requires concrete evidence. Expected schema: {subject"), "{error}");
}

#[test]
fn relanding_unions_by_default_and_replace_supersedes() {
    let (_temp, landing) = review(&["S"]);
    let land = |id: &str, replace: bool| {
        let mut record = json!({"subject":"S","summary":id,"findings":[{"id":id,"claim":format!("claim {id}")}],"evidence":[{"kind":"inspection","summary":format!("looked at {id}")}]});
        if replace { record["replace"] = json!(true); }
        landing.land(record).unwrap();
    };
    land("f1", false);
    land("f2", false);
    let data = landing.assemble(&json!({"records_landed":1})).unwrap();
    let ids = |data: &serde_json::Value| data["findings"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(ids(&data), vec!["f2", "f1"]);
    assert_eq!(data["verification_records"][0]["evidence"].as_array().unwrap().len(), 2);
    land("f3", true);
    let data = landing.assemble(&json!({"records_landed":1})).unwrap();
    assert_eq!(ids(&data), vec!["f3"]);
    assert_eq!(data["verification_records"][0]["evidence"], json!([{"kind":"inspection","summary":"looked at f3"}]));
    assert_eq!(data["verification_records"][0]["summary"], "f3");
    assert!(data["verification_records"][0].get("replace").is_none(), "replace is a write-time instruction, not persisted state");
    land("f4", false);
    assert_eq!(ids(&landing.assemble(&json!({"records_landed":1})).unwrap()), vec!["f4", "f3"]);
}

#[test]
fn replace_on_a_verify_record_drops_the_inherited_failure() {
    let temp = tempfile::tempdir().unwrap();
    let landing = RecordLanding::open(temp.path().into(), "identity".into(), RecordKind::Verify, vec!["S".into()]).unwrap();
    let evidence = json!([{"kind":"test","summary":"ran the suite"}]);
    landing.land(json!({"subject":"S","status":"failed","summary":"first attempt failed","evidence":evidence})).unwrap();
    landing.land(json!({"subject":"S","status":"accepted","summary":"second attempt passed","evidence":evidence})).unwrap();
    let record = &landing.assemble(&json!({"records_landed":1})).unwrap()["verification_records"][0];
    assert_eq!(record["status"], "failed", "without replace the earlier failure is inherited");
    assert_eq!(record["summary"], "first attempt failed; second attempt passed");
    landing.land(json!({"subject":"S","status":"accepted","summary":"second attempt passed","evidence":evidence,"replace":true})).unwrap();
    let record = &landing.assemble(&json!({"records_landed":1})).unwrap()["verification_records"][0];
    assert_eq!(record["status"], "accepted");
    assert_eq!(record["summary"], "second attempt passed");
}

#[test]
fn hint_carries_the_schema_and_the_replace_sentence() {
    let (_temp, landing) = review(&["S"]);
    let hint = landing.hint().unwrap();
    assert!(hint.contains("Re-landing a subject unions with the earlier record; send replace:true to supersede it (use this to withdraw a finding)."), "{hint}");
    assert!(hint.contains(&format!("Schema: {}", schema_hint())), "{hint}");
    assert!(schema_hint().starts_with("{subject: string (one of this call's subjects), findings: [object]*"), "{}", schema_hint());
}
