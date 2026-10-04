// Issue 313: one call's slot that does not read back as that call's record is
// damage, whatever its bytes are, and is quarantined with evidence.

fn slot_store(dir: &tempfile::TempDir) -> WorkflowV2ResultStore {
    WorkflowV2ResultStore::new(dir.path().join("v2"))
}

fn slot_record(id: &str) -> WorkflowV2CallRecord {
    WorkflowV2CallRecord::new(
        "wf-slot",
        call(id),
        1,
        "input".into(),
        WorkflowV2Result::accepted("done"),
        Vec::new(),
    )
}

/// Not JSON, and JSON of the wrong shape: both are damaged, never "empty".
const DAMAGE: [&str; 2] = [
    "{\"call\":",
    r#"{"call":{"id":"slot"},"attempt":"one","input_hash":1,"status":"accepted","result":{}}"#,
];

#[test]
fn a_damaged_slot_is_quarantined_with_evidence_whatever_its_bytes() {
    for damage in DAMAGE {
        let dir = tempfile::tempdir().expect("tmp");
        let store = slot_store(&dir);
        store.save_call_record(&slot_record("slot")).unwrap();
        let path = store.result_path("slot");
        std::fs::write(&path, damage).unwrap();

        let WorkflowV2CallSlot::Damaged { evidence, fresh } =
            store.load_call_slot_healing("slot").unwrap()
        else {
            panic!("{damage}: damage must not read as a record or as absence");
        };

        assert!(fresh, "{damage}");
        assert!(!path.exists(), "{damage}: the slot is emptied");
        let moved = store.root().join(&evidence.quarantined);
        assert_eq!(std::fs::read_to_string(moved).unwrap(), damage);
        assert_eq!(evidence.call_id, "slot");
        assert_eq!(evidence.event, CALL_QUARANTINE_EVENT);
        // The emptied slot stays damaged for the next read, and the
        // directory scans no longer meet the damage.
        assert!(matches!(
            store.load_call_slot_healing("slot").unwrap(),
            WorkflowV2CallSlot::Damaged { fresh: false, .. }
        ));
        assert!(store.load_call_records().unwrap().is_empty(), "{damage}");
    }
}

#[test]
fn another_calls_record_in_the_slot_is_damage() {
    let dir = tempfile::tempdir().expect("tmp");
    let store = slot_store(&dir);
    let foreign = serde_json::to_vec(&slot_record("other")).unwrap();
    std::fs::create_dir_all(store.result_path("slot").parent().unwrap()).unwrap();
    std::fs::write(store.result_path("slot"), foreign).unwrap();

    let slot = store.load_call_slot_healing("slot").unwrap();

    let WorkflowV2CallSlot::Damaged { evidence, .. } = slot else {
        panic!("{slot:?}");
    };
    assert!(evidence.reason.contains("other"), "{evidence:?}");
}

#[test]
fn a_whole_slot_and_an_empty_one_read_as_they_are() {
    let dir = tempfile::tempdir().expect("tmp");
    let store = slot_store(&dir);
    assert!(matches!(
        store.load_call_slot_healing("slot").unwrap(),
        WorkflowV2CallSlot::Empty
    ));
    store.save_call_record(&slot_record("slot")).unwrap();
    assert!(matches!(
        store.load_call_slot_healing("slot").unwrap(),
        WorkflowV2CallSlot::Whole(record) if record.call.id == "slot"
    ));
}

/// A new execution takes the slot back from an earlier quarantine.
#[test]
fn a_new_record_takes_the_slot_back_from_a_quarantine() {
    let dir = tempfile::tempdir().expect("tmp");
    let store = slot_store(&dir);
    std::fs::create_dir_all(store.result_path("slot").parent().unwrap()).unwrap();
    std::fs::write(store.result_path("slot"), DAMAGE[0]).unwrap();
    store.load_call_slot_healing("slot").unwrap();

    store.save_call_record(&slot_record("slot")).unwrap();

    assert!(matches!(
        store.load_call_slot_healing("slot").unwrap(),
        WorkflowV2CallSlot::Whole(_)
    ));
}
