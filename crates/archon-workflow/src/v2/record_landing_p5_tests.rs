#![allow(non_snake_case)]
use archon_workflow::v2::record_landing::{RecordKind, RecordLanding, schema_hint, task_schema};
use serde_json::{Value, json};
fn open(path: &std::path::Path) -> RecordLanding {
    RecordLanding::open(
        path.into(),
        "identity".into(),
        RecordKind::Skeleton,
        vec![],
        false,
    )
    .unwrap()
}
fn task(n: usize) -> Value {
    json!({"task_id":format!("TASK-X-{n:03}"),"file_name":format!("TASK-X-{n:03}.md"),"implements":["REQ-X-001"],"source_sections":["0"]})
}
fn previous() -> Value {
    json!({"schema_version":1,"acceptance_digest":"","tasks":[task(1),task(2)]})
}
fn land(records: &RecordLanding, task: Value) {
    records
        .land(json!({"subject":task["task_id"],"task":task,"replace":true}))
        .unwrap();
}
#[test]
fn P5_20_schema_and_landing_reject_bad_entries() {
    let schema = task_schema();
    let dir = tempfile::tempdir().unwrap();
    let records = open(dir.path());
    for field in ["implements", "source_sections", "blocks"] {
        assert_eq!(schema["properties"][field]["uniqueItems"], true);
        assert_eq!(
            schema["properties"][field]["items"]["pattern"],
            r"^\S(.*\S)?$"
        );
        for values in [json!(["x", "x"]), json!([" x"]), json!([""])] {
            let mut t = task(1);
            t[field] = values;
            assert!(
                records
                    .land(json!({"subject":t["task_id"],"task":t}))
                    .is_err()
            );
        }
    }
    let mut blank = task(1);
    blank["implements"] = json!([" "]);
    let error = records
        .land(json!({"subject":blank["task_id"],"task":blank}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("task.implements[0] is blank"), "{error}");
    assert_eq!(schema["properties"]["depends_on"]["uniqueItems"], true);
    let mut t = task(1);
    t["depends_on"] = json!([{"task_id":"TASK-X-002","ordering_only":true},{"task_id":"TASK-X-002","consumes":[{"artifact_path":"x"}]}]);
    assert!(
        records
            .land(json!({"subject":t["task_id"],"task":t}))
            .is_err()
    );
}
#[test]
fn P5_23_JS_shape_matches_schema() {
    for field in [
        "required_universe",
        "registry_minimum_count",
        "series_overlap_min_rows",
    ] {
        assert!(
            task_schema()["properties"]["deliverable_contracts"]["items"]["properties"]
                .get(field)
                .is_none()
        );
    }
    let source = include_str!("../../../../src/command/workflow_decompose_v1.js");
    let declaration = source
        .split("const SKELETON_SHAPE = ")
        .nth(1)
        .unwrap()
        .split(";\n")
        .next()
        .unwrap();
    let output = std::process::Command::new("node")
        .args(["-e", &format!("process.stdout.write({declaration})")])
        .output()
        .unwrap();
    assert!(output.status.success());
    let shape: Value = serde_json::from_slice(&output.stdout).unwrap();
    fn check(value: &Value, schema: &Value) {
        if let Some(fields) = value.as_object() {
            for (key, value) in fields {
                let property = &schema["properties"][key];
                assert!(!property.is_null(), "{key}");
                check(value, property);
            }
        } else if let Some(items) = value.as_array() {
            for item in items {
                check(item, &schema["items"]);
            }
        }
    }
    check(&shape["tasks"][0], &task_schema());
    let dir = tempfile::tempdir().unwrap();
    let records = open(dir.path());
    let mut t = task(1);
    t["depends_on"] = json!([{"task_id":"TASK-X-002","ordering_only":false,"consumes":[{"artifact_path":"out","instance_source_records_field":"rows","registry_records_field":"entries"}]}]);
    t["deliverable_contracts"] = json!([{"kind":"output","artifact_path":"out","min_instances":1,"instance_source_records_field":"rows","registry_records_field":"entries"}]);
    land(&records, t.clone());
    let composed = records.compose(&json!({"records_landed":1})).unwrap();
    let expected: archon_workflow::task_skeleton::FrozenTask = serde_json::from_value(t).unwrap();
    assert_eq!(
        composed["tasks"][0],
        serde_json::to_value(expected).unwrap()
    );
}
#[test]
fn P5_24_hint_uses_exact_schema() {
    let schema = task_schema();
    let fields = &schema["properties"]["deliverable_contracts"]["items"]["properties"];
    for line in include_str!("../task_universe_deliverable.rs").lines() {
        if let Some(field) = line
            .trim()
            .strip_prefix("pub ")
            .and_then(|s| s.split_once(':'))
        {
            assert!(
                fields.get(field.0).is_some()
                    != [
                        // Deferred from the author hint until host proof is enabled.
                        "implements",
                        "required_universe",
                        "registry_minimum_count",
                        "series_overlap_min_rows"
                    ]
                    .contains(&field.0),
                "missing contract field {}",
                field.0
            );
        }
    }
    assert_eq!(fields["typed_verifier_command"]["type"], "string");
    assert_eq!(fields["closed_weekdays"]["items"]["type"], "integer");
    assert_eq!(
        fields["minimum_count_fields"]["additionalProperties"]["type"],
        "integer"
    );
    assert!(schema_hint(RecordKind::Skeleton).contains(&task_schema().to_string()));
}
#[test]
fn P5_28_patch_preserves_unlisted_record_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let records = open(dir.path());
    records.seed_skeleton(&previous(), false).unwrap();
    let saved: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("record-")
        })
        .map(|path| {
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    let mut changed = task(1);
    changed["implements"] = json!(["REQ-X-002"]);
    land(&records, changed);
    let unchanged = saved
        .iter()
        .filter(|(path, bytes)| std::fs::read(path).unwrap() == *bytes)
        .count();
    assert_eq!(unchanged, 1);
    let composed = records.compose(&json!({"records_landed":2})).unwrap();
    assert_eq!(composed["tasks"][1]["implements"], task(2)["implements"]);
}
#[test]
fn P5_29_first_attempt_empty_store() {
    let dir = tempfile::tempdir().unwrap();
    let records = open(dir.path());
    assert!(records.retained_completion().unwrap().is_none());
    assert_eq!(
        records.compose(&json!({"records_landed":0})).unwrap()["tasks"],
        json!([])
    );
}
#[test]
fn P5_30_frozen_identity_and_draft_identity() {
    let dir = tempfile::tempdir().unwrap();
    let records = open(&dir.path().join("frozen"));
    records.seed_skeleton(&previous(), true).unwrap();
    let mut renamed = task(1);
    renamed["file_name"] = json!("TASK-X-003.md");
    assert!(
        records
            .land(json!({"subject":"TASK-X-001","task":renamed}))
            .unwrap_err()
            .to_string()
            .contains("frozen_field_changed")
    );
    assert!(
        records
            .land(json!({"subject":"TASK-X-001","task":task(3)}))
            .unwrap_err()
            .to_string()
            .contains("missing_frozen_task")
    );
    // A genuinely new task keeps the frozen tasks and remains allowed.
    land(&records, task(3));
    let draft = open(&dir.path().join("draft"));
    draft.seed_skeleton(&previous(), false).unwrap();
    land(&draft, task(3));
    let mut renamed_draft = task(1);
    renamed_draft["file_name"] = json!("TASK-X-001-revised.md");
    land(&draft, renamed_draft);
}
#[test]
fn P5_31_remove_draft_only() {
    let dir = tempfile::tempdir().unwrap();
    let records = open(&dir.path().join("draft"));
    records.seed_skeleton(&previous(), false).unwrap();
    records
        .land(json!({"subject":"TASK-X-001","remove":true}))
        .unwrap();
    assert_eq!(
        records.compose(&json!({"records_landed":1})).unwrap()["tasks"][0]["task_id"],
        "TASK-X-002"
    );
    let error = records
        .land(json!({"subject":"TASK-X-999","remove":true}))
        .unwrap_err()
        .to_string();
    assert!(error.contains("TASK-X-999"), "{error}");
    let frozen = open(&dir.path().join("frozen"));
    frozen.seed_skeleton(&previous(), true).unwrap();
    assert!(
        frozen
            .land(json!({"subject":"TASK-X-001","remove":true}))
            .is_err()
    );
}

#[test]
fn frozen_seed_retains_every_task_with_a_missing_execution_obligation() {
    let dir = tempfile::tempdir().unwrap();
    let records = open(dir.path());
    let mut seed = previous();
    for task in seed["tasks"].as_array_mut().unwrap() {
        task["deliverable_contracts"] =
            json!([{"kind":"output","artifact_path":"out.json","min_instances":0}]);
    }
    records.seed_skeleton(&seed, true).unwrap();
    // remaining(), hint(), compose(), assemble() and retained_completion() all read records().
    assert!(records.remaining().unwrap().is_empty());
    assert!(records.hint().unwrap().contains("Host retained 2 records"));
    assert!(records.retained_completion().unwrap().is_some());
    records.assemble(&json!({"records_landed":2})).unwrap();
    let composed = records.compose(&json!({"records_landed":2})).unwrap();
    let expected: archon_workflow::task_skeleton::TaskSkeleton =
        serde_json::from_value(seed).unwrap();
    assert_eq!(
        composed["tasks"],
        serde_json::to_value(&expected.tasks).unwrap()
    );
    assert!(archon_workflow::task_skeleton::validate_skeleton(&expected, "").is_err());
    let mut weak = task(3);
    weak["deliverable_contracts"] =
        json!([{"kind":"output","artifact_path":"new.json","min_instances":0}]);
    let error = records
        .land(json!({"subject":"TASK-X-003","task":weak}))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("neither a verifier nor a positive instance obligation"),
        "{error}"
    );
    assert!(
        records
            .land(json!({"subject":"TASK-X-001","remove":true}))
            .is_err()
    );
}

#[test]
fn seed_still_skips_tasks_with_invalid_shape() {
    let dir = tempfile::tempdir().unwrap();
    let records = open(dir.path());
    let mut seed = previous();
    seed["tasks"][0]["file_name"] = json!("nested/TASK-X-001.md");
    records.seed_skeleton(&seed, true).unwrap();
    let composed = records.compose(&json!({"records_landed":1})).unwrap();
    assert_eq!(composed["tasks"][0]["task_id"], "TASK-X-002");
}

#[test]
fn P5_authored_completion_is_not_landed() {
    let dir = tempfile::tempdir().unwrap();
    let records = open(dir.path());
    let mut candidate = task(1);
    candidate["status"] = json!("done");
    land(&records, candidate);
    let composed = records.compose(&json!({"records_landed":1})).unwrap();
    let frozen: archon_workflow::task_skeleton::FrozenTask =
        serde_json::from_value(composed["tasks"][0].clone()).unwrap();
    assert_eq!(frozen.status, None);
    assert!(composed["tasks"][0].get("status").is_none());
}
