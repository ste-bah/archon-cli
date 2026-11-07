use super::*;
fn open(root: &std::path::Path, kind: RecordKind) -> RecordLanding {
    RecordLanding::open(
        root.into(),
        "call".into(),
        kind,
        if kind == RecordKind::Skeleton {
            vec![]
        } else {
            vec!["TASK-X-001".into()]
        },
        true,
    )
    .unwrap()
}
fn task() -> Value {
    json!({"task_id":"TASK-X-001","file_name":"TASK-X-001.md","implements":["REQ-1"]})
}
fn seed(store: &RecordLanding) {
    store
        .seed_skeleton(
            &json!({"schema_version":1,"acceptance_digest":"","tasks":[task()]}),
            false,
        )
        .unwrap();
}
// B-store.md:80-84: the real draft removal tool shape leaves no full record.
#[test]
fn record_landing_b1_removal_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path(), RecordKind::Skeleton);
    seed(&store);
    store
        .land(json!({"subject":"TASK-X-001","remove":true}))
        .unwrap();
    drop(store);
    let store = open(dir.path(), RecordKind::Skeleton);
    assert_eq!(store.new_landing_count().unwrap(), 1);
    assert_eq!(
        store
            .compose_reply(&store.retained_reply().unwrap().unwrap())
            .unwrap()["tasks"],
        json!([])
    );
}
// B-store.md:87-92: identical seed tool re-land is not author work on resume.
#[test]
fn record_landing_b2_identical_seed_is_not_work_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path(), RecordKind::Skeleton);
    seed(&store);
    store
        .land(json!({"subject":"TASK-X-001","task":task()}))
        .unwrap();
    drop(store);
    let store = open(dir.path(), RecordKind::Skeleton);
    assert!(!store.has_landed_records().unwrap());
    assert!(store.retained_reply().unwrap().is_none());
}
// B-store.md:90: pending tool rejection and open findings must survive reopen.
#[test]
fn record_landing_b2_pending_and_repair_survive_reopen() {
    for reject in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path(), RecordKind::Skeleton);
        seed(&store);
        store.require_new_landings(true).unwrap();
        if reject {
            let mut bad = task();
            bad["implements"] = json!([]);
            assert!(
                store
                    .land(json!({"subject":"TASK-X-001","task":bad}))
                    .is_err()
            );
        }
        drop(store);
        let store = open(dir.path(), RecordKind::Skeleton);
        assert!(store.compose_reply(&json!({"records_landed":0})).is_err());
        if reject {
            assert!(store.require_repaired().is_err());
        }
    }
}
// B-store.md:94-96: actual record shape with nonempty failed summary.
#[test]
fn record_landing_b3_failed_summary_relanding_is_idempotent() {
    for kind in [RecordKind::Review, RecordKind::Verify] {
        for status in ["failed", "blocked", "needs_review"] {
            let dir = tempfile::tempdir().unwrap();
            let store = open(dir.path(), kind);
            let record = json!({"subject":"TASK-X-001","status":status,"summary":"counterexample remains",
                "findings":[{"claim":"counterexample"}],"evidence":[{"kind":"inspection","summary":"read source"}]});
            store.land(record.clone()).unwrap();
            let before = serde_json::to_value(store.records().unwrap()).unwrap();
            for _ in 0..5 {
                store.land(record.clone()).unwrap();
            }
            assert_eq!(
                serde_json::to_value(store.records().unwrap()).unwrap(),
                before
            );
        }
    }
}

#[test]
fn record_landing_committed_snapshot_recovers_interrupted_record_mutations() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path(), RecordKind::Skeleton);
    seed(&store);
    let mut changed = task();
    changed["source_sections"] = json!(["2"]);
    store
        .land(json!({"subject":"TASK-X-001","task":changed}))
        .unwrap();
    use sha2::{Digest, Sha256};
    let path = dir
        .path()
        .join(format!("record-{:x}.json", Sha256::digest(b"TASK-X-001")));
    // A process dies after changing a record, before committing call-state.json.
    std::fs::remove_file(&path).unwrap();
    drop(store);
    let store = open(dir.path(), RecordKind::Skeleton);
    assert_eq!(store.new_landing_count().unwrap(), 1);
    assert_eq!(
        store.compose_reply(&json!({"records_landed":1})).unwrap()["tasks"][0]["source_sections"],
        json!(["2"])
    );
    // Reverting after reopen compares against the original pre-call digest.
    store
        .land(json!({"subject":"TASK-X-001","task":task()}))
        .unwrap();
    drop(store);
    let store = open(dir.path(), RecordKind::Skeleton);
    assert_eq!(store.new_landing_count().unwrap(), 0);
    assert!(!store.has_landed_records().unwrap());
}

#[test]
fn record_landing_reopen_preserves_removals_when_seed_is_supplied_again() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path(), RecordKind::Skeleton);
    seed(&store);
    store
        .land(json!({"subject":"TASK-X-001","remove":true}))
        .unwrap();
    drop(store);
    let store = open(dir.path(), RecordKind::Skeleton);
    seed(&store);
    assert_eq!(
        store.compose_reply(&json!({"records_landed":1})).unwrap()["tasks"],
        json!([])
    );
}

#[test]
fn record_landing_b3_inherited_failure_summary_is_retained_and_bounded() {
    for kind in [RecordKind::Review, RecordKind::Verify] {
        let dir = tempfile::tempdir().unwrap();
        let store = open(dir.path(), kind);
        let mut record = json!({"subject":"TASK-X-001","status":"failed","summary":"old failure",
            "evidence":[{"kind":"inspection","summary":"read source"}]});
        store.land(record.clone()).unwrap();
        record["status"] = json!("needs_review");
        record["summary"] = json!("latest evidence");
        store.land(record.clone()).unwrap();
        let rows = store.records().unwrap();
        assert_eq!(rows["TASK-X-001"].status, Some(WorkflowV2Status::Failed));
        assert_eq!(rows["TASK-X-001"].summary, "latest evidence\nold failure");
        for _ in 0..5 {
            store.land(record.clone()).unwrap();
        }
        assert_eq!(
            store.records().unwrap()["TASK-X-001"].summary,
            rows["TASK-X-001"].summary
        );
        record["summary"] = json!("latest evidence\nold failure\nnew context");
        store.land(record.clone()).unwrap();
        assert_eq!(
            store.records().unwrap()["TASK-X-001"].summary,
            "latest evidence\nold failure\nnew context"
        );
        let before = serde_json::to_value(store.records().unwrap()).unwrap();
        let mut boundary: StageRecord = serde_json::from_value(record.clone()).unwrap();
        boundary.summary.clear();
        boundary.summary = "x".repeat(32768 - serde_json::to_vec(&boundary).unwrap().len());
        store.validate(&boundary, Admission::NewLanding).unwrap();
        record["summary"] = json!(boundary.summary);
        assert!(store.land(record.clone()).is_err());
        assert_eq!(
            serde_json::to_value(store.records().unwrap()).unwrap(),
            before
        );
        record["summary"] = json!("replacement explanation");
        record["replace"] = json!(true);
        store.land(record).unwrap();
        assert_eq!(
            store.records().unwrap()["TASK-X-001"].status,
            Some(WorkflowV2Status::NeedsReview)
        );
    }
}

#[test]
fn record_landing_read_only_snapshot_leaves_interrupted_store_byte_identical() {
    use sha2::{Digest, Sha256};
    let dir = tempfile::tempdir().unwrap();
    let store = open(dir.path(), RecordKind::Skeleton);
    seed(&store);
    let mut changed = task();
    changed["source_sections"] = json!(["2"]);
    store
        .land(json!({"subject":"TASK-X-001","task":changed}))
        .unwrap();
    let record_path = dir
        .path()
        .join(format!("record-{:x}.json", Sha256::digest(b"TASK-X-001")));
    // Record bytes reached disk but the atomic call-state rename did not.
    std::fs::write(
        &record_path,
        serde_json::to_vec(&json!({"subject":"TASK-X-001","task":task()})).unwrap(),
    )
    .unwrap();
    std::fs::write(dir.path().join("record-uncommitted.json"), b"{}").unwrap();
    let hashes = || -> BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (
                    path.file_name().unwrap().to_str().unwrap().to_owned(),
                    Sha256::digest(std::fs::read(path).unwrap()).to_vec(),
                )
            })
            .collect()
    };
    let before = hashes();
    let reader = RecordLanding::open_read_only(
        dir.path().into(),
        "call".into(),
        RecordKind::Skeleton,
        vec![],
        true,
    )
    .unwrap();
    assert_eq!(
        reader.compose_reply(&json!({"records_landed":1})).unwrap()["tasks"][0]["source_sections"],
        json!(["2"])
    );
    assert_eq!(hashes(), before);
    assert!(
        reader
            .land(json!({"subject":"TASK-X-001","task":task()}))
            .is_err()
    );
    assert_eq!(hashes(), before);
    drop(reader);
    let writer = open(dir.path(), RecordKind::Skeleton);
    assert!(!dir.path().join("record-uncommitted.json").exists());
    assert_eq!(
        writer.compose_reply(&json!({"records_landed":1})).unwrap()["tasks"][0]["source_sections"],
        json!(["2"])
    );
}

#[test]
fn record_landing_read_only_open_does_not_create_missing_store() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("missing");
    assert!(
        RecordLanding::open_read_only(
            root.clone(),
            "call".into(),
            RecordKind::Skeleton,
            vec![],
            true
        )
        .is_err()
    );
    assert!(!root.exists());
}
