use super::*;
use crate::data_store::ahdm_test_support::request;

#[test]
fn validation_report_artifact_uses_prd_schema_version_field() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();

    let artifact: serde_json::Value =
        read_json(&temp.path().join(&record.validation_path)).unwrap();

    assert_eq!(artifact["schema_version"], "archon-trading-validation-v1");
    assert!(artifact.get("schema").is_none());
    assert_eq!(artifact["status"], "passed");
    assert!(artifact["native_interval"].as_bool().unwrap());
    assert!(artifact["production_eligible"].as_bool().unwrap());
    assert!(artifact["checks"].is_array());
    assert!(artifact["summary"].is_object());
    assert!(artifact["validated_at"].is_string());
}

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
fn stooq_short_span_artifact_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = request();
    request.metadata.provider = "stooq".into();
    request.metadata.dataset_id = "stooq-SPY-1D-raw".into();
    request.metadata.canonical_instrument = "SPY".into();
    request.metadata.provider_symbol = "spy.us".into();
    request.metadata.symbol_map = BTreeMap::from([("SPY".into(), "spy.us".into())]);
    request.raw_request = serde_json::json!({"start":"2020-01-01","end":"2024-12-31"});

    let record = lake.store_ohlcv(request).unwrap();
    let dataset = lake
        .load_ohlcv("stooq-SPY-1D-raw", &record.version)
        .unwrap();

    assert!(!dataset.metadata.production_eligible);
    assert_eq!(dataset.record.status, DatasetStatus::Degraded);
    assert!(dataset.metadata.gaps.expected_bars > 1_000);
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
fn yfinance_diagnostic_override_marks_report_and_lists_overridden_issues() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = request();
    request.metadata.dataset_id = "yfinance-BTCUSD-1D-raw".into();
    request.metadata.provider = "yfinance".into();
    request.metadata.provider_symbol = "BTC-USD".into();
    request.metadata.production_eligible = true;
    lake.store_ohlcv(request).unwrap();

    let report = lake
        .backtest_data_gate("yfinance-BTCUSD-1D-raw", "20260101-fixture", true)
        .unwrap();

    assert!(report.diagnostic);
    assert!(!report.promotion_eligible);
    assert_eq!(report.overridden_issues, report.issues);
    assert!(
        report
            .overridden_issues
            .iter()
            .any(|issue| issue.contains("yfinance degraded fallback"))
    );
    let stored = lake
        .load_registry()
        .unwrap()
        .datasets
        .get(&registry_key("yfinance-BTCUSD-1D-raw", "20260101-fixture"))
        .unwrap()
        .clone();
    assert_eq!(stored.status, DatasetStatus::Degraded);
    assert!(!stored.production_eligible);
}

#[test]
fn interrupted_ingest_partial_artifacts_do_not_leave_healthy_registry_entry() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let mut registry = lake.load_registry().unwrap();
    let stored = registry
        .datasets
        .get_mut(&registry_key(&record.dataset_id, &record.version))
        .unwrap();
    std::fs::remove_file(temp.path().join(&stored.normalized_path)).unwrap();
    stored.status = DatasetStatus::Healthy;
    stored.production_eligible = true;
    write_json(&lake.registry_path(), &registry).unwrap();

    assert!(lake.load_registry().is_err());
}

#[test]
fn yfinance_fallback_verify_artifact_passes_and_registry_links_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = request();
    request.metadata.dataset_id = "yfinance-BTCUSD-1D-raw".into();
    request.metadata.provider = "yfinance".into();
    request.metadata.provider_symbol = "BTC-USD".into();
    request.metadata.production_eligible = true;
    request.metadata.quality_status = "passed".into();
    let record = lake.store_ohlcv(request).unwrap();

    let dataset_dir = temp.path().join(&record.dataset_path);
    let verified = TradingDataLake::verify_artifact_dir(&dataset_dir).unwrap();
    assert_eq!(verified.provider, "yfinance");
    assert_eq!(verified.status, DatasetStatus::Degraded);
    assert!(!verified.production_eligible);
    for path in [
        &verified.manifest_path,
        &verified.metadata_path,
        &verified.normalized_path,
        &verified.validation_path,
        &verified.raw_response_path,
        &verified.raw_request_path,
        &verified.redacted_headers_path,
        &verified.provider_notes_path,
    ] {
        assert!(!path.trim().is_empty(), "empty yfinance artifact path");
        assert!(
            temp.path().join(path).exists(),
            "missing yfinance artifact {path}"
        );
    }

    let registry = lake.load_registry().unwrap();
    let registry_record = registry
        .datasets
        .get(&registry_key(&verified.dataset_id, &verified.version))
        .unwrap();
    assert_eq!(registry_record.provider, "yfinance");
    assert_eq!(registry_record.manifest_path, verified.manifest_path);
    assert_eq!(registry_record.metadata_path, verified.metadata_path);
    assert_eq!(registry_record.normalized_path, verified.normalized_path);
    assert_eq!(
        registry_record.raw_response_path,
        verified.raw_response_path
    );
    assert_eq!(registry_record.raw_request_path, verified.raw_request_path);
    assert_eq!(
        registry_record.redacted_headers_path,
        verified.redacted_headers_path
    );
    assert_eq!(
        registry_record.provider_notes_path,
        verified.provider_notes_path
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

fn coverage_matrix(record: &StoredDatasetRecord) -> CoverageMatrix {
    CoverageMatrix {
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
    }
}

fn tradingview_coverage_record(lake: &TradingDataLake) -> StoredDatasetRecord {
    let mut request = request();
    request.metadata.provider = "tradingview".into();
    request.metadata.dataset_id = "tradingview-BTCUSD-1D-raw".into();
    request.metadata.provider_symbol = "BTCUSD".into();
    request.metadata.coverage.expected_bars = AHDM_BACKTEST_MINIMUM_ROWS as u64;
    request.metadata.gaps.expected_bars = AHDM_BACKTEST_MINIMUM_ROWS as u64;
    request.bars = coverage_bars();
    // The live-fetch provenance gate reads the captured request, response body
    // and provider notes, and requires "live", "fetch" and "provider" in each.
    request.raw_request = serde_json::json!({"source":"live provider fetch test capture"});
    request.raw_body = serde_json::to_vec(&serde_json::json!({
        "source": "captured live provider fetch",
        "provider": "tradingview",
        "bar_count": AHDM_BACKTEST_MINIMUM_ROWS,
    }))
    .unwrap();
    request.provider_notes = "captured live provider fetch response".into();
    lake.store_ohlcv(request).unwrap()
}

#[test]
fn typed_coverage_verifier_rejects_a_broken_dataset_checksum_link() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = tradingview_coverage_record(&lake);
    let coverage_path = lake.coverage_dir().join("fixture.json");

    let mut matrix = coverage_matrix(&record);
    write_schema_json(&coverage_path, &matrix).unwrap();
    TradingDataLake::verify_coverage_files(&coverage_path, &lake.registry_path()).unwrap();

    matrix.cells[0].dataset_checksum = Some("fabricated".into());
    write_schema_json(&coverage_path, &matrix).unwrap();
    assert!(TradingDataLake::verify_coverage_files(&coverage_path, &lake.registry_path()).is_err());
}

#[test]
fn typed_coverage_verifier_rejects_quarantined_linked_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = tradingview_coverage_record(&lake);
    let coverage_path = lake.coverage_dir().join("fixture.json");
    let metadata_path = temp.path().join(&record.metadata_path);
    let mut metadata: serde_json::Value = read_json(&metadata_path).unwrap();
    metadata["production_eligible"] = serde_json::json!(false);
    metadata["quality_status"] = serde_json::json!("quarantined_synthetic");
    metadata["quarantined_at"] = serde_json::json!("2026-07-26T07:50:53Z");
    metadata["quarantine_reason"] = serde_json::json!("synthetic placeholder evidence");
    write_json(&metadata_path, &metadata).unwrap();
    write_schema_json(&coverage_path, &coverage_matrix(&record)).unwrap();

    let result = TradingDataLake::verify_coverage_files(&coverage_path, &lake.registry_path());

    assert!(result.is_err());
}

fn coverage_bars() -> Vec<OhlcvBar> {
    // Sized to the production backtest minimum, not the coverage minimum: the
    // AHDM gate rejects anything shorter. 28-day months exceed a year here.
    (0..AHDM_BACKTEST_MINIMUM_ROWS)
        .map(|index| {
            let year = 2026 + (index / 336);
            let month = ((index / 28) % 12) + 1;
            let day = (index % 28) + 1;
            // Not a constant delta: a linear ramp is rejected as placeholder
            // evidence by `bars_have_linear_shape`.
            let cycle = index as f64;
            let close = 10.0 + (cycle / 7.0).sin() * 3.0 + cycle * 0.03;

            OhlcvBar {
                timestamp: format!("{year}-{month:02}-{day:02}T00:00:00Z"),
                open: close,
                high: close + 1.0,
                low: close - 1.0,
                close,
                volume: close * 1_000.0,
            }
        })
        .collect()
}
