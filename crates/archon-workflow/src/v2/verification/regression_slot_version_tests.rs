use super::*;

#[test]
fn old_slot_verdicts_cannot_hide_newly_observed_test_regressions() {
    let temp = tempfile::tempdir().unwrap();
    let store = WorkflowV2ResultStore::new(temp.path().join("v2"));
    let old = store
        .root()
        .join("baseline-tests/regression-slots/example.json");
    std::fs::create_dir_all(old.parent().unwrap()).unwrap();
    let record = RegressionSlot {
        call_id: "example".into(),
        ..Default::default()
    };
    std::fs::write(old, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(slot_record(&store, "example").is_none());
    write_slot(&store, &record).unwrap();
    assert_eq!(slot_record(&store, "example"), Some(record));
}
