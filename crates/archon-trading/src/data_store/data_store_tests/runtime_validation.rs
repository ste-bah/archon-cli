use super::*;

#[test]
fn first_dataset_write_initializes_missing_registry_under_data_root() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    assert!(!lake.registry_path().exists());
    lake.store_ohlcv(request()).unwrap();
    assert_eq!(lake.registry_path(), lake.data_root().join("registry.json"));
    assert!(lake.registry_path().exists());
}

#[test]
fn stored_dataset_has_one_strict_bound_validation_report() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let dataset_dir = temp.path().join(&record.dataset_path);
    let reports: Vec<_> = std::fs::read_dir(&dataset_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() == "validation.json")
        .collect();

    assert_eq!(reports.len(), 1);
    let report_path = reports[0].path();
    assert!(report_path.is_file());
    assert_eq!(report_path, temp.path().join(&record.validation_path));
    let report: ValidationReport = read_json(&report_path).unwrap();
    let metadata: DatasetMetadata = read_json(&temp.path().join(&record.metadata_path)).unwrap();
    let bars = read_jsonl_bars(&temp.path().join(&record.normalized_path)).unwrap();

    assert_eq!(report.dataset_id, record.dataset_id);
    assert_eq!(report.version, record.version);
    assert_eq!(
        report.normalized_sha256,
        normalized_bars_checksum(&bars).unwrap()
    );
    assert_eq!(
        report.normalized_sha256,
        metadata.checksums.normalized_sha256
    );
    assert_eq!(
        report.content_sha256,
        ValidationReport::content_hash(&report.normalized_sha256, &report.checks, &report.summary,)
    );
    assert_eq!(report.summary, validation_summary(&metadata, &bars));
    assert!(report.allows_production());
}
