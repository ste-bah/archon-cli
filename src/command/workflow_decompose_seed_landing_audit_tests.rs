//! C1 (#630 audit C D1): recovery orders calls, preserves landings and gate findings.
use super::super::tests::{gate, reply};
use super::super::*;
use archon_workflow::v2::record_landing::{RecordKind, RecordLanding};
use serde_json::json;

fn candidate(section: &str) -> Value {
    json!({"schema_version":1,"acceptance_digest":"","tasks":[{
        "task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-1"],"source_sections":[section]
    }]})
}

#[test]
fn seed_skeleton_retained_result_survives_artifact_only_gate_refusal() {
    let initial = candidate("initial").to_string();
    let repaired = candidate("repaired").to_string();
    let findings = vec![json!({"text":"repair source","remediation_scope":"skeleton"})];
    let mut refused = reply("skeleton-author-2", "01:02:00", "");
    refused.status = WorkflowV2Status::Failed;
    refused.result.status = WorkflowV2Status::Failed;
    refused.result.data = json!({"retainedLandings":true,"content":repaired,"invalid_answer":true});
    let records = vec![
        reply("skeleton-author-1", "01:00:00", &initial),
        gate(
            "freeze-skeleton",
            "01:01:00",
            &initial,
            findings.clone(),
            false,
        ),
        refused,
        gate(
            "freeze-skeleton",
            "01:03:00",
            &repaired,
            vec![json!({"text":"refused artifact","remediation_scope":"candidate_artifact"})],
            false,
        ),
    ];
    let derived = derive(&records, &[], &BTreeMap::new()).unwrap();
    let SubjectSeed::Artifact {
        candidate,
        candidate_call,
        gate,
    } = &derived.subjects["skeleton"]
    else {
        panic!("artifact")
    };
    assert_eq!(candidate, &repaired);
    assert_eq!(candidate_call, "skeleton-author-2");
    assert_eq!(gate.as_ref().unwrap().findings, findings);
}

#[test]
fn seed_skeleton_older_and_invalidated_stores_cannot_replace_newer_reply() {
    let temp = tempfile::tempdir().unwrap();
    let mut invalidated = reply("skeleton-author-4", "01:04:00", "");
    invalidated.invalidated_by = Some("replacement".into());
    let records = vec![
        reply("skeleton-author-2", "01:02:00", "prose"),
        reply(
            "skeleton-author-3",
            "01:03:00",
            &candidate("latest").to_string(),
        ),
        invalidated,
    ];
    for call in ["skeleton-author-2", "skeleton-author-4"] {
        let path = temp.path().join("stage-records").join(call).join("session");
        let store = RecordLanding::open(
            path.clone(),
            call.into(),
            RecordKind::Skeleton,
            vec![],
            false,
        )
        .unwrap();
        std::fs::write(path.join("author.json"), serde_json::to_vec(call).unwrap()).unwrap();
        store
            .land(json!({"subject":"TASK-X-001","task":candidate("older")["tasks"][0]}))
            .unwrap();
    }
    let mut derived = derive(&records, &[], &BTreeMap::new()).unwrap();
    artifact::retain_skeleton(&mut derived, &records, temp.path()).unwrap();
    let SubjectSeed::Artifact {
        candidate: held,
        candidate_call,
        ..
    } = &derived.subjects["skeleton"]
    else {
        panic!("artifact")
    };
    assert_eq!(candidate_call, "skeleton-author-3");
    assert_eq!(
        serde_json::from_str::<Value>(held).unwrap(),
        candidate("latest")
    );
}
