//! Resume regressions for malformed skeleton replies and retained repairs.
use super::tests::{gate, reply};
use super::*;
use serde_json::json;

#[test]
fn seed_skeleton_syntax_refusal_keeps_the_last_judged_candidate_and_findings() {
    let candidate = json!({"schema_version":1,"acceptance_digest":"","tasks":[]}).to_string();
    let findings: Vec<_> = (1..=9)
        .map(|n| json!({"text":format!("repair task {n}"),"remediation_scope":"skeleton"}))
        .collect();
    let records = vec![
        reply("skeleton-author-1", "01:00:00", &candidate),
        gate(
            "freeze-skeleton",
            "01:01:00",
            &candidate,
            findings.clone(),
            false,
        ),
        reply("skeleton-author-4", "01:02:00", "Repaired tasks."),
        gate(
            "freeze-skeleton",
            "01:03:00",
            "Repaired tasks.",
            vec![json!({"text":"Syntax at line 1","remediation_scope":"candidate_artifact"})],
            false,
        ),
    ];
    let derived = derive(&records, &[], &BTreeMap::new()).unwrap();
    let SubjectSeed::Artifact {
        candidate: held,
        gate,
        ..
    } = &derived.subjects["skeleton"]
    else {
        panic!("artifact seed")
    };
    assert_eq!(held, &candidate);
    assert_eq!(gate.as_ref().unwrap().findings, findings);
    assert_eq!(derived.author_ordinals["skeleton"], 4);
}

#[test]
fn seed_skeleton_real_resume_keeps_retained_tasks_and_nine_gate_findings() {
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/skeleton_resume");
    let temp = tempfile::tempdir().unwrap();
    fn copy(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.path().is_dir() {
                copy(&entry.path(), &dest);
            } else {
                std::fs::copy(entry.path(), dest).unwrap();
            }
        }
    }
    copy(&fixture, temp.path());
    let root = temp.path();
    let mtimes: BTreeMap<String, u64> =
        serde_json::from_slice(&std::fs::read(root.join("mtimes.json")).unwrap()).unwrap();
    for (path, nanos) in mtimes {
        std::fs::File::options()
            .write(true)
            .open(root.join(path))
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_nanos(nanos))
            .unwrap();
    }
    let records = archon_workflow::WorkflowV2ResultStore::new(root.join("v2"))
        .load_call_records()
        .unwrap();
    let syntax_refusal = records
        .iter()
        .find(|record| record.call.id.starts_with("fef38c2a"))
        .unwrap();
    assert_eq!(
        syntax_refusal.result.data["gateEnvelope"]["policy_findings"][0]["remediation_scope"],
        "candidate_artifact"
    );
    assert!(
        !records
            .iter()
            .any(|record| record.call.id.starts_with("1c2d990c"))
    );
    let mut derived = derive(&records, &[], &BTreeMap::new()).unwrap();
    let events = (0..2)
        .map(|part| std::fs::read_to_string(root.join(format!("events-{part}.jsonl"))).unwrap())
        .collect::<String>();
    assert!(!events.is_empty());
    artifact::retain_skeleton(&mut derived, &records, &root.join("v2")).unwrap();
    let SubjectSeed::Artifact {
        candidate,
        candidate_call,
        gate,
    } = &derived.subjects["skeleton"]
    else {
        panic!("artifact seed")
    };
    let gate = gate.as_ref().unwrap();
    assert_eq!(gate.findings.len(), 9);
    assert!(gate.call_id.starts_with("2b6a852b"));
    let expected_gate = records.iter().find(|r| r.call.id == gate.call_id).unwrap();
    assert_eq!(
        serde_json::to_value(&gate.findings).unwrap(),
        expected_gate.result.data["gateEnvelope"]["policy_findings"]
    );
    assert!(
        gate.findings
            .iter()
            .all(|finding| finding["remediation_scope"] == "skeleton")
    );
    let mut expected = BTreeMap::new();
    let store = std::fs::read_dir(root.join("v2/stage-records"))
        .unwrap()
        .map(|entry| entry.unwrap())
        .find(|dir| dir.file_name().to_string_lossy().starts_with("dfd03edb"))
        .unwrap();
    for session in std::fs::read_dir(store.path())
        .unwrap()
        .map(|entry| entry.unwrap())
    {
        for path in std::fs::read_dir(session.path())
            .unwrap()
            .map(|entry| entry.unwrap())
        {
            if path.file_name().to_string_lossy().starts_with("record-") {
                let record: Value =
                    serde_json::from_slice(&std::fs::read(path.path()).unwrap()).unwrap();
                expected.insert(
                    record["subject"].as_str().unwrap().to_owned(),
                    record["task"].clone(),
                );
            }
        }
    }
    let actual: Value = serde_json::from_str(candidate).unwrap();
    let tasks: BTreeMap<_, _> = actual["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| (task["task_id"].as_str().unwrap().to_owned(), task.clone()))
        .collect();
    assert_eq!(candidate_call, "skeleton-author-4");
    assert_eq!(tasks, expected);
    assert_eq!(tasks.len(), 19);
}

#[test]
fn seed_skeleton_uses_the_latest_landed_store_and_skips_a_later_seed_only_store() {
    use archon_workflow::v2::record_landing::{RecordKind, RecordLanding};
    let temp = tempfile::tempdir().unwrap();
    let candidate = json!({"schema_version":1,"acceptance_digest":"","tasks":[
        {"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-1"],"source_sections":["1"]}
    ]});
    let findings: Vec<_> = (1..=9)
        .map(|n| json!({"text":format!("repair task {n}"),"remediation_scope":"skeleton"}))
        .collect();
    let records = vec![
        reply("skeleton-author-1", "01:00:00", &candidate.to_string()),
        gate(
            "freeze-skeleton",
            "01:01:00",
            &candidate.to_string(),
            findings.clone(),
            false,
        ),
        reply("skeleton-author-4", "01:02:00", "Repaired tasks."),
        reply("skeleton-author-5", "01:03:00", ""),
    ];
    for call in ["skeleton-author-4", "skeleton-author-5"] {
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
        store.seed_skeleton(&candidate, false).unwrap();
        if call == "skeleton-author-4" {
            let mut task = candidate["tasks"][0].clone();
            task["source_sections"] = json!(["2"]);
            store
                .land(json!({"subject":"TASK-X-001","task":task}))
                .unwrap();
        }
    }
    let mut derived = derive(&records, &[], &BTreeMap::new()).unwrap();
    artifact::retain_skeleton(&mut derived, &records, temp.path()).unwrap();
    let SubjectSeed::Artifact {
        candidate,
        candidate_call,
        gate,
    } = &derived.subjects["skeleton"]
    else {
        panic!("artifact seed")
    };
    assert_eq!(candidate_call, "skeleton-author-4");
    let candidate: Value = serde_json::from_str(candidate).unwrap();
    assert_eq!(candidate["tasks"][0]["source_sections"], json!(["2"]));
    assert_eq!(gate.as_ref().unwrap().findings, findings);
}

#[test]
fn seed_skeleton_redispatched_author_uses_newest_store_and_skips_unreadable_store() {
    use archon_workflow::v2::record_landing::{RecordKind, RecordLanding};
    let temp = tempfile::tempdir().unwrap();
    let candidate = json!({"schema_version":1,"acceptance_digest":"","tasks":[
        {"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-1"]}
    ]});
    let records = vec![
        reply("skeleton-author-1", "01:00:00", &candidate.to_string()),
        reply("skeleton-author-4", "01:02:00", "Repaired tasks."),
    ];
    for (session, section, seconds) in [("old", "2", 1), ("new", "3", 2)] {
        let path = temp.path().join("stage-records/author").join(session);
        let store = RecordLanding::open(
            path.clone(),
            "author".into(),
            RecordKind::Skeleton,
            vec![],
            false,
        )
        .unwrap();
        std::fs::write(
            path.join("author.json"),
            serde_json::to_vec("skeleton-author-4").unwrap(),
        )
        .unwrap();
        let mut task = candidate["tasks"][0].clone();
        task["source_sections"] = json!([section]);
        store
            .land(json!({"subject":"TASK-X-001","task":task}))
            .unwrap();
        std::fs::File::options()
            .write(true)
            .open(path.join("contract.json"))
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds))
            .unwrap();
    }
    let broken = temp.path().join("stage-records/author/broken");
    std::fs::create_dir_all(&broken).unwrap();
    std::fs::write(broken.join("contract.json"), "broken").unwrap();
    let mut derived = derive(&records, &[], &BTreeMap::new()).unwrap();
    artifact::retain_skeleton(&mut derived, &records, temp.path()).unwrap();
    let SubjectSeed::Artifact {
        candidate,
        candidate_call,
        ..
    } = &derived.subjects["skeleton"]
    else {
        panic!("artifact")
    };
    assert_eq!(candidate_call, "skeleton-author-4");
    assert_eq!(
        serde_json::from_str::<Value>(candidate).unwrap()["tasks"][0]["source_sections"],
        json!(["3"])
    );
}
