//! Issue-35/36: landing rejections name the offending path and the full schema; `replace:true` supersedes an earlier landing.
//! Issue-37: a reduce finding keeps its own task ids and must name one; only a map subject stands in for a missing task.
use super::record_landing::{schema_hint, RecordKind, RecordLanding};
use serde_json::json;

fn review(subjects: &[&str]) -> (tempfile::TempDir, RecordLanding) { open(subjects, true) }
fn reduce(subject: &str) -> (tempfile::TempDir, RecordLanding) { open(&[subject], false) }
fn open(subjects: &[&str], subjects_are_tasks: bool) -> (tempfile::TempDir, RecordLanding) {
    let temp = tempfile::tempdir().unwrap();
    let landing = RecordLanding::open(temp.path().join("records"), "identity".into(), RecordKind::Review, subjects.iter().map(|s| s.to_string()).collect(), subjects_are_tasks).unwrap();
    (temp, landing)
}
fn evidence() -> serde_json::Value { json!([{"kind":"inspection","summary":"read the implementation"}]) }

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
    let landing = RecordLanding::open(temp.path().into(), "identity".into(), RecordKind::Verify, vec!["S".into()], true).unwrap();
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

#[test]
fn map_subject_stands_in_for_a_finding_without_task_fields() {
    let (_temp, landing) = review(&["TASK-1"]);
    landing.land(json!({"subject":"TASK-1","findings":[{"id":"f1","claim":"boundary unchecked"}],"evidence":evidence()})).unwrap();
    let data = landing.assemble(&json!({"records_landed":1})).unwrap();
    assert_eq!(data["findings"][0]["canonical_task_ids"], json!(["TASK-1"]));
}

#[test]
fn reduce_finding_keeps_its_own_task_id_instead_of_the_call_id() {
    let (_temp, landing) = reduce("some-reduce");
    landing.land(json!({"subject":"some-reduce","findings":[
        {"id":"f1","claim":"missing retry","task_id":"TASK-2"},
        {"id":"f2","claim":"shared helper drift","task_ids":["TASK-2","TASK-3"]},
        {"id":"f3","claim":"stale doc","canonical_task_ids":["TASK-4"],"task_id":"TASK-9"}],"evidence":evidence()})).unwrap();
    let data = landing.assemble(&json!({"records_landed":1})).unwrap();
    assert_eq!(data["findings"][0]["canonical_task_ids"], json!(["TASK-2"]));
    assert_eq!(data["findings"][1]["canonical_task_ids"], json!(["TASK-2","TASK-3"]));
    assert_eq!(data["findings"][2]["canonical_task_ids"], json!(["TASK-4"]), "an explicit canonical_task_ids wins over task_id");
    assert!(data["findings"].as_array().unwrap().iter().all(|f| f["canonical_task_ids"] != json!(["some-reduce"])));
}

#[test]
fn reduce_finding_without_a_task_is_rejected_with_the_schema() {
    let (_temp, landing) = reduce("some-reduce");
    let error = landing.land(json!({"subject":"some-reduce","findings":[{"id":"f1","title":"TASK-2 leaks a handle"}],"evidence":evidence()})).unwrap_err().to_string();
    assert!(error.contains("each finding must name the task that owns the fix (task_id, or task_ids/canonical_task_ids) or set attributable_to_task:false"), "{error}");
    assert!(error.contains("Expected schema: {subject"), "{error}");
    let error = landing.land(json!({"subject":"some-reduce","findings":[{"id":"f1","claim":"x","task_ids":[]}],"evidence":evidence()})).unwrap_err().to_string();
    assert!(error.contains("attributable_to_task"), "an empty task_ids names nothing: {error}");
    assert!(schema_hint().contains("must name the task that owns the fix via task_id, or task_ids/canonical_task_ids, or set attributable_to_task:false"), "{}", schema_hint());
}

#[test]
fn unattributable_reduce_finding_lands_without_a_task_stamp() {
    let (_temp, landing) = reduce("some-reduce");
    landing.land(json!({"subject":"some-reduce","findings":[{"id":"f1","claim":"two tasks disagree on the wire format","attributable_to_task":false}],"evidence":evidence()})).unwrap();
    let data = landing.assemble(&json!({"records_landed":1})).unwrap();
    assert!(data["findings"][0].get("canonical_task_ids").is_none(), "{}", data["findings"][0]);
    assert_eq!(data["findings"][0]["attributable_to_task"], json!(false));
}

// Issue-38: a skeleton record must land one full task entry; the tool says what that is and rejects an ownerless stub.
fn skeleton() -> (tempfile::TempDir, RecordLanding) {
    let temp = tempfile::tempdir().unwrap();
    let landing = RecordLanding::open(temp.path().join("records"), "identity".into(), RecordKind::Skeleton, vec![], false).unwrap();
    (temp, landing)
}

#[test]
fn skeleton_record_with_a_full_task_lands_and_assembles_under_tasks() {
    let (_temp, landing) = skeleton();
    let task = json!({"task_id":"TASK-CORE-002","file_name":"TASK-CORE-002.md",
        "depends_on":[{"task_id":"TASK-CORE-001","consumes":[{"artifact_path":"out/schema.json"}],"ordering_only":false}],
        "blocks":["TASK-CORE-003"],"implements":["REQ-4"],
        "deliverable_contracts":[{"kind":"report","artifact_path":"out/report.json","min_instances":1}]});
    landing.land(json!({"subject":"TASK-CORE-002","summary":"reads the schema, writes the report","task":task})).unwrap();
    let data = landing.assemble(&json!({"records_landed":1})).unwrap();
    assert_eq!(data["tasks"].as_array().unwrap().len(), 1);
    let landed = &data["tasks"][0];
    for field in ["task_id", "file_name", "depends_on", "blocks", "implements"] { assert_eq!(landed[field], task[field], "{field}"); }
    for field in ["kind", "artifact_path", "min_instances"] { assert_eq!(landed["deliverable_contracts"][0][field], task["deliverable_contracts"][0][field], "{field}"); }
}

#[test]
fn skeleton_stub_without_obligations_or_deliverables_is_rejected_with_the_schema() {
    let (_temp, landing) = skeleton();
    let error = landing.land(json!({"subject":"TASK-CORE-002","summary":"depends on TASK-CORE-001","task":{"task_id":"TASK-CORE-002","file_name":"TASK-CORE-002.md","depends_on":[],"blocks":[],"implements":[],"deliverable_contracts":[]}})).unwrap_err().to_string();
    assert!(error.contains("a skeleton task must name the PRD obligations it implements and/or the artifacts it delivers"), "{error}");
    assert!(error.contains("implements"), "{error}");
    assert!(error.contains("Expected schema:"), "{error}");
    assert!(landing.assemble(&json!({"records_landed":0})).unwrap()["tasks"].as_array().unwrap().is_empty(), "the stub must not be retained");
    let error = landing.land(json!({"subject":"TASK-CORE-002","task":{"task_id":"TASK-CORE-002","file_name":"TASK-CORE-002.md"}})).unwrap_err().to_string();
    assert!(error.contains("cannot be owned or verified"), "omitted arrays are the same stub: {error}");
    landing.land(json!({"subject":"TASK-CORE-002","task":{"task_id":"TASK-CORE-002","file_name":"TASK-CORE-002.md","deliverable_contracts":[{"kind":"report","artifact_path":"out/report.json"}]}})).unwrap();
}

#[test]
fn schema_hint_describes_the_skeleton_task_entry() {
    let hint = schema_hint();
    assert!(hint.contains("task (skeleton records only): {task_id: TASK-<DOMAIN>-<NNN>, file_name: <task_id>.md, depends_on: [{task_id, consumes: [{artifact_path}], ordering_only: bool}]*, blocks: [task_id]*, implements: [PRD obligation id]*, deliverable_contracts: [{kind, artifact_path, min_instances: int}]*; implements and/or deliverable_contracts must be non-empty}"), "{hint}");
    assert!(hint.contains("implements") && hint.contains("deliverable_contracts"), "{hint}");
    assert!(!hint.contains('\n'), "the hint is one line: {hint}");
}
