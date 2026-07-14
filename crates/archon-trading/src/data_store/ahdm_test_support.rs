#[cfg(test)]
pub(super) fn ahdm_position_size(account_equity: f64, entry: f64, stop: f64) -> Option<f64> {
    let risk_per_unit = (entry - stop).abs();
    if !account_equity.is_finite()
        || !entry.is_finite()
        || !stop.is_finite()
        || account_equity <= 0.0
        || entry <= 0.0
        || risk_per_unit <= 0.0
    {
        return None;
    }
    Some(((account_equity * 0.005) / risk_per_unit).min((account_equity * 0.01) / entry))
}

use super::*;
use crate::data_lake::{CoverageWindow, CurrentSnapshot, DataType, GapSummary};

#[test]
fn coverage_matrix_persists_latest_history_and_readable_markdown() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    store_complete_trading_core_coverage(&lake);
    persist_trading_core_snapshots(&lake, 1_781_049_600);

    let matrix = lake
        .write_coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into())
        .unwrap();

    assert_eq!(matrix.cells.len(), 30);
    assert!(matrix.gaps.is_empty());
    assert!(matrix.cells.iter().all(|cell| {
        cell.available
            && cell.dataset_id.is_some()
            && cell.production_eligible
            && cell.row_count > 0
    }));
    assert!(matrix.cells.iter().any(|cell| {
        cell.canonical_instrument == "SPY" && cell.timeframe == "1D" && cell.available
    }));
    assert!(lake.coverage_dir().join("latest.json").exists());
    assert!(lake.coverage_dir().join("latest.md").exists());
    assert!(
        lake.coverage_dir()
            .join("history/2026-06-10T00_00_00Z.json")
            .exists()
    );
    let markdown = std::fs::read_to_string(lake.coverage_dir().join("latest.md")).unwrap();
    assert!(markdown.contains("| SPY | 1D | tradingview | SPY | true | true | true | passed |"));
}

#[test]
fn coverage_matrix_fails_closed_when_required_registry_cells_are_missing() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(spy_request()).unwrap();
    persist_spy_snapshot(&lake, 1_781_049_600);

    let result = lake.write_coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into());

    assert!(matches!(
        result,
        Err(DataStoreError::InvalidMetadata(message))
            if message.contains("coverage matrix incomplete for trading-core-v1")
                && message.contains("ES:1W")
                && message.contains("no provider-native validated registry dataset")
    ));
    assert!(!lake.coverage_dir().join("latest.json").exists());
}

#[test]
fn coverage_cell_unavailable_reason_includes_selected_provider_and_capability() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let matrix = lake
        .coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into())
        .unwrap();
    let cell = matrix
        .cells
        .iter()
        .find(|cell| cell.canonical_instrument == "ES" && cell.timeframe == "1W")
        .unwrap();
    assert!(!cell.available);
    assert_eq!(cell.selected_provider, "tradingview");
    assert!(
        cell.fallback_reason
            .as_deref()
            .unwrap()
            .contains("no provider-native validated registry dataset")
    );
}

#[test]
fn coverage_matrix_refuses_false_positive_non_native_cell() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = spy_request();
    request.metadata.native_interval = false;
    request.metadata.production_eligible = true;
    lake.store_ohlcv(request).unwrap();
    persist_spy_snapshot(&lake, 1_781_049_600);

    let matrix = lake
        .coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into())
        .unwrap();
    let cell = matrix
        .cells
        .iter()
        .find(|cell| cell.canonical_instrument == "SPY" && cell.timeframe == "1D")
        .unwrap();

    assert!(!cell.available);
    assert!(cell.dataset_id.is_none());
    assert!(
        cell.fallback_reason
            .as_deref()
            .unwrap()
            .contains("native_interval=false")
    );
}

#[test]
fn coverage_matrix_refuses_false_positive_failed_validation_cell() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(spy_request()).unwrap();
    persist_spy_snapshot(&lake, 1_781_049_600);
    let validation_path = temp.path().join(&record.validation_path);
    let mut report: ValidationReport = read_json(&validation_path).unwrap();
    report.status = ValidationStatus::Failed;
    report.production_eligible = false;
    write_json(&validation_path, &report).unwrap();

    let matrix = lake
        .coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into())
        .unwrap();
    let cell = matrix
        .cells
        .iter()
        .find(|cell| cell.canonical_instrument == "SPY" && cell.timeframe == "1D")
        .unwrap();

    assert!(!cell.available);
    assert!(
        cell.fallback_reason
            .as_deref()
            .unwrap()
            .contains("validation status is not passed")
    );
}

#[test]
fn coverage_matrix_refuses_false_positive_checksum_mismatch_cell() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(spy_request()).unwrap();
    persist_spy_snapshot(&lake, 1_781_049_600);
    let metadata_path = temp.path().join(&record.metadata_path);
    let mut metadata: DatasetMetadata = read_json(&metadata_path).unwrap();
    metadata.checksum = "wrong-checksum".into();
    write_json(&metadata_path, &metadata).unwrap();

    let matrix = lake
        .coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into())
        .unwrap();
    let cell = matrix
        .cells
        .iter()
        .find(|cell| cell.canonical_instrument == "SPY" && cell.timeframe == "1D")
        .unwrap();

    assert!(!cell.available);
    assert!(
        cell.fallback_reason
            .as_deref()
            .unwrap()
            .contains("checksum mismatch")
    );
}

#[test]
fn coverage_matrix_refuses_false_positive_missing_artifact_cell() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(spy_request()).unwrap();
    persist_spy_snapshot(&lake, 1_781_049_600);
    std::fs::remove_file(temp.path().join(&record.manifest_path)).unwrap();

    let matrix = lake
        .coverage_matrix("trading-core-v1", "2026-06-10T00:00:00Z".into())
        .unwrap();
    let cell = matrix
        .cells
        .iter()
        .find(|cell| cell.canonical_instrument == "SPY" && cell.timeframe == "1D")
        .unwrap();

    assert!(!cell.available);
    assert!(
        cell.fallback_reason
            .as_deref()
            .unwrap()
            .contains("missing artifact")
    );
}

#[test]
fn backtest_gate_enforces_required_raw_artifact_contract() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    std::fs::remove_file(temp.path().join(&record.provider_notes_path)).unwrap();

    let result = lake.backtest_data_gate("manual-BTCUSD-1D-raw", "20260101-fixture", false);
    assert!(matches!(
        result,
        Err(DataStoreError::InvalidMetadata(message))
            if message.contains("raw/provider-notes.md")
    ));
    assert!(temp.path().join(record.raw_path).exists());
}

#[test]
fn d42_backtest_gate_refuses_manual_dataset_claiming_native_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request()).unwrap();

    let result = lake.backtest_data_gate("manual-BTCUSD-1D-raw", "20260101-fixture", false);

    assert!(matches!(
        result,
        Err(DataStoreError::InvalidMetadata(message))
            if message.contains("manual datasets cannot satisfy provider-native")
    ));
}

#[test]
fn registry_schema_v2_preserves_v1_readability_and_blocks_unknown_schema() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let existing = lake.store_ohlcv(request()).unwrap();
    let mut registry = lake.load_registry().unwrap();
    registry.schema_version = REGISTRY_SCHEMA_V1.into();
    write_json(&lake.registry_path(), &registry).unwrap();

    let mut next = request();
    next.metadata.version = "20260105-fixture".into();
    next.created_at = "2026-01-05T00:00:00Z".into();
    lake.store_ohlcv(next).unwrap();

    let migrated = lake.load_registry().unwrap();
    assert_eq!(migrated.schema_version, REGISTRY_SCHEMA_V2);
    assert!(
        migrated
            .datasets
            .contains_key(&registry_key(&existing.dataset_id, &existing.version))
    );

    let mut unknown = migrated;
    unknown.schema_version = "archon-trading-data-registry-v999".into();
    write_json(&lake.registry_path(), &unknown).unwrap();
    assert!(matches!(
        lake.load_registry(),
        Err(DataStoreError::InvalidRegistrySchema(schema))
            if schema == "archon-trading-data-registry-v999"
    ));
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
fn validation_fails_when_native_interval_metadata_is_not_provider_supported() {
    let mut metadata = request().metadata;
    metadata.provider = "stooq".into();
    metadata.dataset_id = "stooq-BTCUSD-15-raw".into();
    metadata.timeframe = "15".into();
    metadata.symbol_map = BTreeMap::from([("BTCUSD".into(), "BTCUSD".into())]);
    metadata.native_interval = true;
    metadata.production_eligible = true;
    metadata.coverage.start = "2026-01-01T00:00:00Z".into();
    metadata.coverage.end = "2026-01-02T00:00:00Z".into();
    metadata.checksum = "checksum".into();

    let report = validation_report(&metadata, &request().bars, "now".into());
    assert_eq!(report.status, ValidationStatus::Failed);
    assert!(report.checks.iter().any(|check| {
        check.id == "metadata.native_interval" && check.status == ValidationStatus::Failed
    }));
    assert!(!report.production_eligible);
}

#[test]
fn validation_report_is_registry_referenced_and_passed_for_valid_native_bars() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let registry = lake.load_registry().unwrap();
    let stored = registry
        .datasets
        .get(&registry_key(&record.dataset_id, &record.version))
        .unwrap();
    let report: ValidationReport = read_json(&temp.path().join(&stored.validation_path)).unwrap();

    assert_eq!(stored.validation_path, record.validation_path);
    assert_eq!(report.status, ValidationStatus::Passed);
    assert!(report.production_eligible);
}

#[test]
fn invalid_fixture_bars_are_rejected_before_storage() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = request();
    request.bars[1].timestamp = request.bars[0].timestamp.clone();

    let result = lake.store_ohlcv(request);
    assert!(matches!(
        result,
        Err(DataStoreError::InvalidOhlcv(message))
            if message.contains("DuplicateTimestamp")
    ));
}

fn request() -> StoreOhlcvRequest {
    StoreOhlcvRequest {
        metadata: DatasetMetadata {
            schema_version: "archon-trading-dataset-v2".into(),
            dataset_id: "manual-BTCUSD-1D-raw".into(),
            version: "20260101-fixture".into(),
            canonical_instrument: "BTCUSD".into(),
            asset_class: "crypto".into(),
            provider: "manual".into(),
            provider_symbol: "BTCUSD".into(),
            timeframe: "1D".into(),
            native_interval: true,
            production_eligible: true,
            price_basis: "raw".into(),
            session: "24x7".into(),
            data_type: DataType::Ohlcv,
            symbol_map: BTreeMap::from([("BTCUSD".into(), "BTCUSD".into())]),
            timezone: "UTC".into(),
            adjustment: "raw".into(),
            license: "research".into(),
            coverage: CoverageWindow {
                start: String::new(),
                end: String::new(),
                expected_bars: 2,
                observed_bars: 0,
            },
            gaps: GapSummary {
                missing_bars: 0,
                expected_bars: 2,
            },
            checksum: String::new(),
            checksums: DatasetChecksums::default(),
            paths: DatasetArtifactPaths::default(),
            source: DatasetSourceMetadata::default(),
            quality_status: "passed".into(),
            created_at: String::new(),
            optional: false,
        },
        bars: vec![
            bar("2026-01-01T00:00:00Z", 10.0),
            bar("2026-01-02T00:00:00Z", 11.0),
        ],
        raw_body: b"raw".to_vec(),
        raw_format: OhlcvFormat::Csv,
        raw_request: serde_json::json!({"source":"test"}),
        redacted_headers: serde_json::json!({}),
        provider_notes: "test fixture".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }
}

fn spy_request() -> StoreOhlcvRequest {
    core_coverage_request("SPY", "1D")
}

fn core_coverage_request(instrument: &str, timeframe: &str) -> StoreOhlcvRequest {
    let mut request = request();
    request.metadata.provider = "tradingview".into();
    request.metadata.canonical_instrument = instrument.into();
    request.metadata.provider_symbol = provider_symbol(instrument, "tradingview");
    request.metadata.dataset_id = format!("tradingview-{instrument}-{timeframe}-raw");
    request.metadata.asset_class = if instrument.ends_with("USDT") {
        "crypto"
    } else {
        "equity"
    }
    .into();
    request.metadata.timeframe = timeframe.into();
    request.metadata.symbol_map = BTreeMap::from([(
        instrument.into(),
        provider_symbol(instrument, "tradingview"),
    )]);
    request
}

fn store_complete_trading_core_coverage(lake: &TradingDataLake) {
    for instrument in trading_core_instruments() {
        for timeframe in trading_core_timeframes() {
            lake.store_ohlcv(core_coverage_request(&instrument, &timeframe))
                .unwrap();
        }
    }
}

fn persist_trading_core_snapshots(lake: &TradingDataLake, captured_at_unix_seconds: i64) {
    for instrument in trading_core_instruments() {
        persist_snapshot(lake, &instrument, captured_at_unix_seconds);
    }
}

fn persist_spy_snapshot(lake: &TradingDataLake, captured_at_unix_seconds: i64) {
    persist_snapshot(lake, "SPY", captured_at_unix_seconds);
}

fn persist_snapshot(lake: &TradingDataLake, instrument: &str, captured_at_unix_seconds: i64) {
    lake.persist_snapshot(
        CurrentSnapshot {
            provider: "tradingview".into(),
            canonical_instrument: instrument.into(),
            provider_symbol: provider_symbol(instrument, "tradingview"),
            captured_at_unix_seconds,
            payload: serde_json::json!({"last": 500.0}),
        },
        1_781_049_600,
    )
    .unwrap();
}

fn bar(timestamp: &str, close: f64) -> OhlcvBar {
    OhlcvBar {
        timestamp: timestamp.into(),
        open: close,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: close * 1_000.0,
    }
}
