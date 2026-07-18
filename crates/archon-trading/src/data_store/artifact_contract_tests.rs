use super::*;
use crate::data_store::ahdm_test_support::request;

#[test]
fn artifact_contract_healthy_registry_entries_point_to_existing_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let registry = lake.load_registry().unwrap();
    let stored = registry
        .datasets
        .get(&registry_key(&record.dataset_id, &record.version))
        .unwrap();

    assert_eq!(stored.status, DatasetStatus::Healthy);
    assert_eq!(stored.provider, "manual");
    assert_eq!(stored.symbol, "BTCUSD");
    assert_eq!(stored.timeframe, "1D");
    assert!(stored.native_interval);
    assert!(stored.production_eligible);
    for path in [
        &stored.metadata_path,
        &stored.validation_path,
        &stored.manifest_path,
        &stored.normalized_path,
        &stored.raw_response_path,
        &stored.raw_request_path,
        &stored.redacted_headers_path,
        &stored.provider_notes_path,
    ] {
        assert!(!path.trim().is_empty(), "empty artifact path");
        assert!(temp.path().join(path).exists(), "missing artifact {path}");
    }
}

#[test]
fn raw_response_contract_supports_prd_filenames() {
    assert_eq!(raw_filename(OhlcvFormat::Json), "response.json");
    assert_eq!(raw_filename(OhlcvFormat::Csv), "response.csv");
    assert_eq!(raw_filename(OhlcvFormat::Zip), "response.zip");
    assert_eq!(raw_filename(OhlcvFormat::Txt), "response.txt");
}

#[test]
fn derived_resampled_diagnostic_candles_are_not_production_eligible() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = request();
    request.metadata.dataset_id = "manual-BTCUSD-1D-resampled".into();
    request.metadata.price_basis = "resampled".into();
    request.metadata.production_eligible = true;
    lake.store_ohlcv(request).unwrap();

    let dataset = lake
        .load_ohlcv("manual-BTCUSD-1D-resampled", "20260101-fixture")
        .unwrap();
    assert!(!dataset.metadata.production_eligible);
    assert_eq!(dataset.metadata.quality_status, "diagnostic");
    let report: ValidationReport =
        read_json(&temp.path().join(&dataset.record.validation_path)).unwrap();
    assert_eq!(report.status, ValidationStatus::Failed);
    assert!(!report.production_eligible);
    assert!(report.checks.iter().any(|check| {
        check.id == "metadata.not_derived_or_resampled" && check.status == ValidationStatus::Failed
    }));
    let gate = lake
        .backtest_data_gate("manual-BTCUSD-1D-resampled", "20260101-fixture", true)
        .unwrap();
    assert!(!gate.promotion_eligible);
    assert!(
        gate.issues
            .iter()
            .any(|issue| { issue.contains("derived/resampled diagnostic candles") })
    );
}

#[test]
fn yfinance_fallback_artifacts_are_degraded_and_never_production_eligible() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = request();
    request.metadata.dataset_id = "yfinance-BTCUSD-1D-raw".into();
    request.metadata.provider = "yfinance".into();
    request.metadata.provider_symbol = "BTC-USD".into();
    request.metadata.production_eligible = true;
    request.metadata.quality_status = "passed".into();
    lake.store_ohlcv(request).unwrap();

    let dataset = lake
        .load_ohlcv("yfinance-BTCUSD-1D-raw", "20260101-fixture")
        .unwrap();
    assert!(!dataset.metadata.production_eligible);
    assert_eq!(dataset.metadata.quality_status, "degraded");
    assert_eq!(dataset.record.status, DatasetStatus::Degraded);
    assert!(!dataset.record.production_eligible);
    let report: ValidationReport =
        read_json(&temp.path().join(&dataset.record.validation_path)).unwrap();
    assert_eq!(report.status, ValidationStatus::Failed);
    assert!(!report.production_eligible);
    assert!(report.checks.iter().any(|check| {
        check.id == "metadata.not_yfinance_fallback" && check.status == ValidationStatus::Failed
    }));
    let gate = lake
        .backtest_data_gate("yfinance-BTCUSD-1D-raw", "20260101-fixture", true)
        .unwrap();
    assert!(!gate.promotion_eligible);
    assert!(
        gate.issues
            .iter()
            .any(|issue| issue.contains("yfinance degraded fallback"))
    );
}

#[test]
fn typed_artifact_verifier_accepts_pipeline_output_and_rejects_fabricated_validation() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let dataset_dir = temp.path().join(&record.dataset_path);

    let verified = TradingDataLake::verify_artifact_dir(&dataset_dir).unwrap();
    assert_eq!(verified.checksum, record.checksum);

    let validation_path = temp.path().join(&record.validation_path);
    let mut fabricated: serde_json::Value = read_json(&validation_path).unwrap();
    fabricated["status"] = serde_json::json!("passed");
    fabricated["production_eligible"] = serde_json::json!(true);
    fabricated["checks"][0]["status"] = serde_json::json!("failed");
    write_json(&validation_path, &fabricated).unwrap();

    assert!(TradingDataLake::verify_artifact_dir(&dataset_dir).is_err());
}

#[test]
fn registry_load_degrades_record_when_native_interval_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let mut registry = lake.load_registry().unwrap();
    let stored = registry
        .datasets
        .get_mut(&registry_key(&record.dataset_id, &record.version))
        .unwrap();
    stored.native_interval = false;
    stored.production_eligible = true;
    stored.status = DatasetStatus::Healthy;
    let manifest_path = temp.path().join(&stored.manifest_path);
    write_json(&manifest_path, stored).unwrap();
    write_json(&lake.registry_path(), &registry).unwrap();

    let reconciled = lake.load_registry().unwrap();
    let stored = reconciled
        .datasets
        .get(&registry_key(&record.dataset_id, &record.version))
        .unwrap();

    assert!(!stored.production_eligible);
    assert_eq!(stored.status, DatasetStatus::Degraded);
}

#[test]
fn registry_load_degrades_record_when_production_eligible_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let mut registry = lake.load_registry().unwrap();
    let stored = registry
        .datasets
        .get_mut(&registry_key(&record.dataset_id, &record.version))
        .unwrap();
    stored.production_eligible = false;
    stored.status = DatasetStatus::Healthy;
    let manifest_path = temp.path().join(&stored.manifest_path);
    write_json(&manifest_path, stored).unwrap();
    write_json(&lake.registry_path(), &registry).unwrap();

    let reconciled = lake.load_registry().unwrap();
    let stored = reconciled
        .datasets
        .get(&registry_key(&record.dataset_id, &record.version))
        .unwrap();

    assert!(!stored.production_eligible);
    assert_eq!(stored.status, DatasetStatus::Degraded);
}

#[test]
fn typed_coverage_verifier_rejects_a_broken_dataset_checksum_link() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let coverage_path = lake.coverage_dir().join("fixture.json");
    let mut matrix = CoverageMatrix {
        schema_version: "fixture-coverage-v1".into(),
        generated_at: "2026-01-01T00:00:00Z".into(),
        instruments: vec![record.symbol.clone()],
        timeframes: vec![record.timeframe.clone()],
        cells: vec![CoverageCell {
            canonical_instrument: record.symbol.clone(),
            timeframe: record.timeframe.clone(),
            selected_provider: record.provider.clone(),
            provider_symbol: record.symbol.clone(),
            dataset_id: Some(record.dataset_id.clone()),
            version: Some(record.version.clone()),
            dataset_checksum: Some(record.checksum.clone()),
            available: true,
            native_interval: true,
            production_eligible: true,
            quality_status: "passed".into(),
            row_count: record.bars as u64,
            coverage_start: record.coverage_start.clone(),
            coverage_end: record.coverage_end.clone(),
            fallback_reason: None,
        }],
        gaps: Vec::new(),
    };
    write_schema_json(&coverage_path, &matrix).unwrap();
    TradingDataLake::verify_coverage_files(&coverage_path, &lake.registry_path()).unwrap();

    matrix.cells[0].dataset_checksum = Some("fabricated".into());
    write_schema_json(&coverage_path, &matrix).unwrap();
    assert!(TradingDataLake::verify_coverage_files(&coverage_path, &lake.registry_path()).is_err());
}
