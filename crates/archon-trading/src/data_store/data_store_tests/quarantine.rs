use super::*;

/// Registry status is derived on every load, so quarantine metadata must remain
/// authoritative even when validation.json still describes a prior pass.
#[test]
fn a_quarantined_dataset_is_never_reconciled_back_to_healthy() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let key = registry_key(&record.dataset_id, &record.version);
    let before = lake.status().unwrap();
    assert_eq!(before.datasets[&key].status, DatasetStatus::Healthy);
    assert!(before.datasets[&key].production_eligible);

    let metadata_path = temp.path().join(&record.metadata_path);
    let mut metadata: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).unwrap()).unwrap();
    metadata["quarantined_at"] = serde_json::json!("2026-08-09T09:00:00Z");
    metadata["quarantine_reason"] = serde_json::json!("provenance unprovable");
    std::fs::write(
        &metadata_path,
        serde_json::to_string_pretty(&metadata).unwrap(),
    )
    .unwrap();

    let after = lake.status().unwrap();
    assert_eq!(after.datasets[&key].status, DatasetStatus::Degraded);
    assert!(!after.datasets[&key].production_eligible);
    let again = lake.status().unwrap();
    assert_eq!(again.datasets[&key].status, DatasetStatus::Degraded);
}
