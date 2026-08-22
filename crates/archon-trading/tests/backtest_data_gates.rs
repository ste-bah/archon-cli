use archon_trading::data_lake::{
    BacktestDataGateReport, BacktestDatasetRef, BacktestGateDecision, BacktestGateIssueClass,
    BacktestRunMode, CoverageWindow, DataType, DatasetArtifactPaths, DatasetChecksums,
    DatasetMetadata, DatasetSourceMetadata, GapSummary, backtest_gate_allows_promotion,
};
use archon_trading::data_store::{DataStoreError, StoreOhlcvRequest, TradingDataLake};
use archon_trading::ohlcv::{OhlcvBar, OhlcvFormat};
use std::collections::BTreeMap;

const DATASET_ID: &str = "manual-BTCUSD-1D-raw";
const VERSION: &str = "20260101-gate";
const ROWS: usize = 400;

#[test]
fn complete_registered_version_is_production_allowed() {
    let (_temp, lake, record) = published(request());
    let report = lake
        .backtest_data_gate(&record.dataset_id, &record.version, false)
        .unwrap();
    assert!(report.issues.is_empty(), "{report:#?}");
    assert!(report.promotion_eligible);
    assert_eq!(
        lake.load_ohlcv_for_backtest(DATASET_ID, VERSION)
            .unwrap()
            .bars
            .len(),
        ROWS
    );
}

#[test]
fn every_section_29_condition_fails_closed() {
    let cases: [fn(&mut StoreOhlcvRequest); 3] = [
        |request| {
            request.metadata.native_interval = false;
            request.metadata.production_eligible = false;
            request.metadata.quality_status = "degraded".into();
        },
        |request| {
            request.metadata.production_eligible = false;
            request.metadata.quality_status = "degraded".into();
        },
        |request| {
            request.metadata.quality_status = "degraded".into();
            request.metadata.production_eligible = false;
        },
    ];
    for mutate in cases {
        let mut candidate = request();
        mutate(&mut candidate);
        let (_temp, lake, _) = published(candidate);
        assert!(matches!(
            lake.load_ohlcv_for_backtest(DATASET_ID, VERSION),
            Err(DataStoreError::InvalidMetadata(_))
        ));
    }
}

#[test]
fn current_artifact_integrity_is_required() {
    let (temp, lake, record) = published(request());
    std::fs::OpenOptions::new()
        .append(true)
        .open(temp.path().join(&record.normalized_path))
        .unwrap()
        .write_all(b"{}\n")
        .unwrap();
    let error = lake
        .backtest_data_gate(DATASET_ID, VERSION, false)
        .unwrap_err();
    assert!(
        matches!(error, DataStoreError::InvalidMetadata(message) if message.contains("checksum_mismatch"))
    );
}

#[test]
fn only_registered_id_and_version_are_accepted() {
    let (temp, lake, _) = published(request());
    let absolute = temp.path().to_string_lossy().into_owned();
    for (dataset_id, version) in [
        ("../manual-BTCUSD-1D-raw", VERSION),
        (DATASET_ID, "../20260101-gate"),
        (absolute.as_str(), VERSION),
        (DATASET_ID, "other-version"),
    ] {
        assert!(matches!(
            lake.load_ohlcv_for_backtest(dataset_id, version),
            Err(DataStoreError::MissingDataset(_)) | Err(DataStoreError::InvalidMetadata(_))
        ));
    }
}

#[test]
fn structural_issues_reject_diagnostic_mode() {
    let (temp, lake, record) = published(request());
    std::fs::remove_file(temp.path().join(&record.validation_path)).unwrap();
    assert!(matches!(
        lake.backtest_data_gate(DATASET_ID, VERSION, true),
        Err(DataStoreError::InvalidMetadata(message)) if message.contains("raw_artifact_missing")
    ));
    assert!(matches!(
        lake.load_ohlcv_for_diagnostic(DATASET_ID, VERSION),
        Err(DataStoreError::InvalidMetadata(_))
    ));
}

#[test]
fn diagnostic_round_trip_never_promotes() {
    let mut degraded = request();
    degraded.metadata.production_eligible = false;
    degraded.metadata.quality_status = "degraded".into();
    let (_temp, lake, _) = published(degraded);
    let report = lake.backtest_data_gate(DATASET_ID, VERSION, true).unwrap();
    assert!(report.diagnostic);
    assert!(!report.promotion_eligible);
    assert!(!report.overridden_issues.is_empty());
    assert!(
        report
            .issues
            .iter()
            .all(|issue| issue.class == BacktestGateIssueClass::Policy)
    );
    let round_trip: archon_trading::data_lake::BacktestDataGateReport =
        serde_json::from_slice(&serde_json::to_vec(&report).unwrap()).unwrap();
    assert_eq!(round_trip, report);
    assert!(!round_trip.promotion_eligible);
}

#[test]
fn strict_reference_and_report_mutations_fail_closed() {
    for value in [
        "",
        "latest",
        "../version",
        "id/version",
        "id\\version",
        "id\n",
    ] {
        assert!(
            !BacktestDatasetRef {
                dataset_id: value.into(),
                version: VERSION.into(),
            }
            .is_strict()
        );
    }
    let (_temp, lake, _) = published(request());
    let report = lake.backtest_data_gate(DATASET_ID, VERSION, false).unwrap();
    assert_eq!(report.decision, BacktestGateDecision::ProductionAllowed);
    assert!(report.is_consistent());
    assert!(backtest_gate_allows_promotion(&report));

    let mutations: Vec<Box<dyn Fn(&mut BacktestDataGateReport)>> = vec![
        Box::new(|value| value.schema_version = "wrong".into()),
        Box::new(|value| value.mode = BacktestRunMode::ExploratoryDiagnostic),
        Box::new(|value| value.decision = BacktestGateDecision::DiagnosticOnly),
        Box::new(|value| value.classification = "exploratory_diagnostic_non_promotable".into()),
        Box::new(|value| value.diagnostic = true),
        Box::new(|value| value.promotion_eligible = false),
    ];
    for mutate in mutations {
        let mut changed = report.clone();
        mutate(&mut changed);
        assert!(!backtest_gate_allows_promotion(&changed));
    }
}

#[test]
fn gate_errors_do_not_disclose_artifact_paths() {
    let (temp, lake, record) = published(request());
    std::fs::remove_file(temp.path().join(&record.validation_path)).unwrap();
    let error = lake
        .backtest_data_gate(DATASET_ID, VERSION, false)
        .unwrap_err();
    let rendered = format!("{error:?}");
    assert!(!rendered.contains(temp.path().to_string_lossy().as_ref()));
    assert!(!rendered.contains(&record.validation_path));
}

#[test]
fn all_candle_readers_are_gated() {
    let (temp, lake, record) = published(request());
    std::fs::write(temp.path().join(&record.raw_response_path), b"tampered").unwrap();
    assert!(lake.load_ohlcv(DATASET_ID, VERSION).is_err());
    assert!(lake.load_ohlcv_for_diagnostic(DATASET_ID, VERSION).is_err());
    assert!(lake.load_ohlcv_for_backtest(DATASET_ID, VERSION).is_err());
}

fn published(
    request: StoreOhlcvRequest,
) -> (
    tempfile::TempDir,
    TradingDataLake,
    Box<archon_trading::data_store::StoredDatasetRecord>,
) {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = Box::new(lake.store_ohlcv(request).unwrap());
    (temp, lake, record)
}

fn request() -> StoreOhlcvRequest {
    let bars = (0..ROWS)
        .map(|index| {
            let year = 2026 + index / 336;
            let month = (index / 28) % 12 + 1;
            let day = index % 28 + 1;
            let cycle = index as f64;
            bar(
                &format!("{year}-{month:02}-{day:02}T00:00:00Z"),
                100.0 + (cycle / 7.0).sin() * 3.0 + cycle * 0.03,
            )
        })
        .collect();
    StoreOhlcvRequest {
        metadata: metadata(),
        bars,
        raw_body: serde_json::to_vec(&serde_json::json!({
            "source": "captured live provider fetch",
            "provider": "manual",
            "bar_count": ROWS
        }))
        .unwrap(),
        raw_format: OhlcvFormat::Json,
        raw_request: serde_json::json!({
            "source": "captured live provider fetch",
            "native_lineage_evidence": {
                "observation": {
                    "dataset_id": DATASET_ID,
                    "version": VERSION,
                    "provider": "manual",
                    "canonical_instrument": "BTCUSD",
                    "provider_symbol": "BTCUSD",
                    "timeframe": "1D",
                    "retrieved_at": "2026-01-01T00:00:00Z",
                    "exact_native_interval": true,
                    "complete": true
                },
                "lineage": {
                    "aggregated": false,
                    "resampled": false,
                    "downsampled": false,
                    "upsampled": false,
                    "interpolated": false,
                    "synthesized": false
                }
            }
        }),
        redacted_headers: serde_json::json!({}),
        provider_notes: "captured live provider fetch response".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }
}

fn metadata() -> DatasetMetadata {
    DatasetMetadata {
        schema_version: "archon-trading-dataset-v1".into(),
        dataset_id: DATASET_ID.into(),
        version: VERSION.into(),
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
            expected_bars: ROWS as u64,
            observed_bars: 0,
        },
        gaps: GapSummary {
            missing_bars: 0,
            expected_bars: ROWS as u64,
        },
        checksum: String::new(),
        checksums: DatasetChecksums::default(),
        paths: DatasetArtifactPaths::default(),
        source: DatasetSourceMetadata::default(),
        quality_status: "passed".into(),
        created_at: String::new(),
        optional: false,
    }
}

fn bar(timestamp: &str, close: f64) -> OhlcvBar {
    OhlcvBar {
        timestamp: timestamp.into(),
        open: close - 0.2,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: 1_000.0 + close,
    }
}

use std::io::Write as _;
