//! Duplicate keys must follow the actual reader, not Value's collapsed map.
use super::*;
use crate::command::workflow_freeze_candidate::acceptance_candidate_for_validation;
use archon_workflow::task_set_contract::AcceptanceContract;
use archon_workflow::task_skeleton::TaskSkeleton;

fn assert_duplicate(document: &str, pointer: &str) {
    let error = serde_json::from_str::<TaskSkeleton>(document).unwrap_err();
    assert!(error.to_string().contains("duplicate field"), "{error}");
    let defects = element_shape_defects(document.as_bytes(), &TASK_SHAPE);
    assert_eq!(defects.len(), 1, "{pointer}: {defects:?}; serde: {error}");
    assert_eq!(defects[0].identity.subject, pointer);
}

#[test]
fn workflow_freeze_round12_duplicate_task_id() {
    assert_duplicate(
        r#"{"schema_version":1,"acceptance_digest":"d","tasks":[{"task_id":"T","task_id":"T","file_name":"f"}]}"#,
        "tasks/0/task_id",
    );
}

#[test]
fn workflow_freeze_round12_duplicate_deliverable_kind() {
    assert_duplicate(
        r#"{"schema_version":1,"acceptance_digest":"d","tasks":[{"task_id":"T","file_name":"f","deliverable_contracts":[{"kind":"file","kind":"file","artifact_path":"a"}]}]}"#,
        "tasks/0/deliverable_contracts/0/kind",
    );
}

#[test]
fn workflow_freeze_round12_duplicate_root_schema_version() {
    assert_duplicate(
        r#"{"schema_version":1,"schema_version":1,"acceptance_digest":"d","tasks":[]}"#,
        "schema_version",
    );
}

#[test]
fn workflow_freeze_round12_duplicate_and_missing_repairs_decrease() {
    let steps = [
        r#"{"schema_version":1,"acceptance_digest":"d","tasks":[{"task_id":"T","task_id":"T","file_name":"f"},{"task_id":"U"}]}"#,
        r#"{"schema_version":1,"acceptance_digest":"d","tasks":[{"task_id":"T","task_id":"T","file_name":"f"},{"task_id":"U","file_name":"g"}]}"#,
        r#"{"schema_version":1,"acceptance_digest":"d","tasks":[{"task_id":"T","file_name":"f"},{"task_id":"U","file_name":"g"}]}"#,
    ];
    let counts: Vec<_> = steps
        .iter()
        .map(|s| element_shape_defects(s.as_bytes(), &TASK_SHAPE).len())
        .collect();
    assert_eq!(counts, [2, 1, 0]);
    assert!(serde_json::from_str::<TaskSkeleton>(steps[2]).is_ok());
}

#[test]
fn workflow_freeze_round12_acceptance_duplicates_follow_assembly() {
    for document in [
        r#"{"entries":[{"id":"A","id":"A","criterion":"c","check":{"kind":"command","kind":"command","command":"true","cwd":"repo_root"}}],"entries":[{"id":"A","criterion":"c","check":{"kind":"command","command":"true","cwd":"repo_root"}}]}"#,
        r#"{"schema_version":1,"schema_version":1,"prd":{"path":"","digest":""},"gap_policy":{},"acceptance":[{"id":"A","criterion":"c","check":{"kind":"command","command":"true","command":"true","cwd":"repo_root"},"judgment":{"verdict":"accepted","reason":"","counterexample":"","host_call_id":"h"}}]}"#,
    ] {
        let bytes = acceptance_candidate_for_validation(document.as_bytes()).unwrap();
        assert!(serde_json::from_slice::<AcceptanceContract>(&bytes).is_ok());
        assert!(element_shape_defects(document.as_bytes(), &ENTRY_SHAPE).is_empty());
    }
}

#[test]
fn workflow_freeze_round12_maps_and_ignored_fields_allow_valid_duplicates() {
    let document = br#"{"schema_version":1,"acceptance_digest":"d","unused":{"a/b~c":0,"a/b~c":1},"unused":{},"tasks":[{"task_id":"T","file_name":"f","deliverable_contracts":[{"kind":"file","artifact_path":"a","minimum_count_fields":{"a/b~c":2,"a/b~c":3}}]}]}"#;
    assert!(serde_json::from_slice::<TaskSkeleton>(document).is_ok());
    assert!(element_shape_defects(document, &TASK_SHAPE).is_empty());
}

#[test]
fn workflow_freeze_round12_overwritten_invalid_values_remain_visible() {
    for (document, expected) in [
        (
            r#"{"schema_version":false,"schema_version":1,"acceptance_digest":"d","tasks":[]}"#,
            vec!["schema_version", "schema_version"],
        ),
        (
            r#"{"schema_version":1,"acceptance_digest":"d","tasks":[{"task_id":"T","file_name":"f","deliverable_contracts":[{"kind":"file","artifact_path":"a","minimum_count_fields":{"a/b~c":false,"a/b~c":2}}]}]}"#,
            vec!["tasks/0/deliverable_contracts/0/minimum_count_fields/a~1b~0c"],
        ),
    ] {
        assert!(serde_json::from_str::<TaskSkeleton>(document).is_err());
        let defects = element_shape_defects(document.as_bytes(), &TASK_SHAPE);
        let subjects: Vec<_> = defects
            .iter()
            .map(|d| d.identity.subject.as_str())
            .collect();
        assert_eq!(subjects, expected);
    }
    // Removing either duplicate copy repairs one of two distinct identities.
    let invalid_only = br#"{"schema_version":false,"acceptance_digest":"d","tasks":[]}"#;
    let valid_only = br#"{"schema_version":1,"acceptance_digest":"d","tasks":[]}"#;
    assert_eq!(element_shape_defects(invalid_only, &TASK_SHAPE).len(), 1);
    assert!(element_shape_defects(valid_only, &TASK_SHAPE).is_empty());
}

#[test]
fn workflow_freeze_round12_duplicate_pointers_count_each_repeat_and_survive_positional_structs() {
    // Round 14: each repeated copy is its own defect. Merging them let a
    // deletion change the reader's error without changing the count.
    let document = br#"[1,"d",[["T","f",[],[],[],[{"kind":"file","kind":"file","kind":"file","artifact_path":"a","artifact_path":"a"}]]]]"#;
    assert!(serde_json::from_slice::<TaskSkeleton>(document).is_err());
    let defects = element_shape_defects(document, &TASK_SHAPE);
    let subjects: Vec<_> = defects
        .iter()
        .map(|d| d.identity.subject.as_str())
        .collect();
    assert_eq!(
        subjects,
        ["2/0/5/0/artifact_path", "2/0/5/0/kind", "2/0/5/0/kind"]
    );
}

#[test]
fn workflow_freeze_round12_unresolved_tag_messages_are_conditional() {
    let defects = element_shape_defects(
        br#"{"entries":[{"id":"A","criterion":"c","check":{}}]}"#,
        &ENTRY_SHAPE,
    );
    assert_eq!(defects.len(), 9);
    for defect in defects {
        assert!(
            defect.message.contains("resolve entries/0/check/kind"),
            "{}",
            defect.message
        );
        assert!(
            defect.message.contains("command, floor"),
            "{}",
            defect.message
        );
    }
}
