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
fn seed_skeleton_artifact_refusal_recovers_prior_judged_candidate() {
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
        ..
    } = &derived.subjects["skeleton"]
    else {
        panic!("artifact")
    };
    assert_eq!(candidate, &initial);
    assert_eq!(candidate_call, &records[1].call.id);
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

// Exercise R18's actual Rust seed through the JavaScript resume loop.
pub(super) fn assert_gate_first(derived: &Derived) {
    let payload = json!({"subjects": derived.subjects, "author_ordinals": derived.author_ordinals});
    let output = std::process::Command::new("node")
        .arg("-e")
        .arg("require(process.argv[1]).assertGateFirst(JSON.parse(process.argv[2])).catch(e => { console.error(e); process.exitCode = 1; })")
        .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/command/workflow_decompose_seed_gate_first_test.cjs"))
        .arg(payload.to_string())
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn seed_skeleton_accepted_reply_interrupted_before_gate_is_unjudged() {
    for retained in [false, true] {
        let initial = candidate("initial").to_string();
        let changed = candidate("changed").to_string();
        let mut accepted = reply("skeleton-author-2", "01:02:00", &changed);
        accepted.result.data["retainedLandings"] = json!(retained);
        let records = vec![
            reply("skeleton-author-1", "01:00:00", &initial),
            gate(
                "freeze-skeleton",
                "01:01:00",
                &initial,
                vec![json!({"text":"old finding","remediation_scope":"skeleton"})],
                false,
            ),
            accepted,
        ];
        let derived = derive(&records, &[], &BTreeMap::new()).unwrap();
        let SubjectSeed::Artifact {
            candidate,
            gate,
            retained_landings,
            ..
        } = &derived.subjects["skeleton"]
        else {
            panic!("artifact")
        };
        assert_eq!(candidate, &changed);
        assert!(gate.is_none());
        assert_eq!(*retained_landings, retained);
    }
}

#[test]
fn seed_skeleton_receipt_hint_counts_new_landings_truthfully() {
    let temp = tempfile::tempdir().unwrap();
    let store = RecordLanding::open(
        temp.path().into(),
        "hint".into(),
        RecordKind::Skeleton,
        vec![],
        false,
    )
    .unwrap();
    store.seed_skeleton(&candidate("initial"), false).unwrap();
    let hint = store.hint().unwrap();
    assert!(hint.contains("{\"records_landed\":N}"), "{hint}");
    assert!(hint.contains("N is the new landings count from the last receipt"));
    assert!(hint.contains("If no task needs a change, your final reply is exactly {\"records_landed\":0}; otherwise land each new or changed task, then reply with {\"records_landed\":N}, where N is the new landings count from the last receipt (seeds do not count)."), "{hint}");
    assert!(store.compose_reply(&json!({"records_landed":0})).is_ok());
    store
        .land(json!({"subject":"TASK-X-001","task":candidate("changed")["tasks"][0]}))
        .unwrap();
    let hint = store.hint().unwrap();
    assert!(hint.contains("exactly {\"records_landed\":1}"), "{hint}");
    assert!(store.compose_reply(&json!({"records_landed":1})).is_ok());
}
