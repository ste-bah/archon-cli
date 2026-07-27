use super::*;
use crate::data_lake::{CoverageWindow, DataType, GapSummary};
use crate::data_store::ahdm_test_support::ahdm_position_size;
use crate::data_store::data_store_ahdm_helpers_tests::{backtest_config, bar};

#[test]
fn ahdm_evidence_inventory_and_citations_fail_closed_for_gaps() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.write_ahdm_evidence_inventory("2026-06-10T00:00:00Z")
        .unwrap();
    let citations: serde_json::Value =
        read_json(&lake.ahdm_strategy_root().join("evidence/citations.json")).unwrap();
    assert_eq!(citations["schema"], "archon-ahdm-citations-v1");
    assert!(citations.get("schema_version").is_none());
    assert_eq!(
        citations["promotion_policy"]["hypotheses_barred_from_promotion_until_cited"],
        true
    );
    assert_eq!(
        citations["inventory_gate"]["prerequisite_evidence"]["coverage_matrix_checked_before_rule_extraction"],
        true
    );
    let rules = citations["rules"].as_array().unwrap();
    assert!(rules.iter().all(|rule| {
        let cited = rule["status"] == "cited" && rule["citation"].is_object();
        let hypothesis = rule["status"] == "hypothesis"
            && rule["citation"].is_null()
            && rule["promotion_allowed"] == false;
        cited || hypothesis
    }));
    assert!(
        rules
            .iter()
            .filter(|rule| rule["status"] == "cited")
            .all(|rule| {
                let citation = rule["citation"].as_object().unwrap();
                citation["reviewed_source_excerpt"] == true
                    && !citation["locator"]
                        .as_str()
                        .unwrap()
                        .ends_with(":rule-inventory")
                    && !citation["excerpt"].as_str().unwrap().is_empty()
            })
    );
    assert_eq!(
        read_json::<serde_json::Value>(&lake.ahdm_strategy_root().join("citations.json")).unwrap(),
        citations
    );
    let inventory = std::fs::read_to_string(
        lake.ahdm_strategy_root()
            .join("evidence/kb-rule-inventory.md"),
    )
    .unwrap();
    assert!(inventory.contains("AC-AHDM-004"));
    assert!(inventory.contains("source_excerpt="));
    assert!(!inventory.contains("trading-elliott-wave"));
}

mod prerequisites;
mod strategy_spec;

#[test]
fn ahdm_position_sizing_is_capped_and_invalid_inputs_no_trade() {
    assert_eq!(ahdm_position_size(10_000.0, 100.0, 95.0), Some(1.0));
    assert_eq!(ahdm_position_size(10_000.0, 100.0, 99.9), Some(1.0));
    assert_eq!(ahdm_position_size(10_000.0, 100.0, 100.0), None);
    assert_eq!(ahdm_position_size(10_000.0, 0.0, 95.0), None);
}

mod pine;
#[test]
fn ahdm_native_backtest_writes_replayable_manifest_parity_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request()).unwrap();
    let dir = lake
        .run_ahdm_native_backtest(
            "run-1",
            "polygon-BTCUSD-1D-raw",
            "20260101-fixture",
            backtest_config(),
            1.0,
            "2026-06-10T00:00:00Z",
        )
        .unwrap();
    assert!(dir.join("config.json").exists());
    assert!(dir.join("report.json").exists());
    assert!(dir.join("trades.jsonl").exists());
    assert!(dir.join("equity_curve.jsonl").exists());
    assert!(dir.join("adversarial-review.md").exists());

    let config: serde_json::Value = read_json(&dir.join("config.json")).unwrap();
    let report: serde_json::Value = read_json(&dir.join("report.json")).unwrap();
    assert_eq!(config["schema"], "archon-ahdm-backtest-config-v1");
    assert_eq!(report["schema"], "archon-ahdm-backtest-report-v1");
    assert_eq!(config["dataset"]["dataset_id"], "polygon-BTCUSD-1D-raw");
    assert_eq!(config["dataset"]["native_interval"], true);
    assert_eq!(report["diagnostic"], false);
    assert_eq!(report["promotion_eligible"], true);
    assert_eq!(config["manifest_hash"], report["shared_rule_manifest_hash"]);
    assert!(report["report"]["metrics"]["expectancy"].is_number());
}

#[test]
fn ahdm_readiness_report_records_failed_gates_and_residual_gap_schema() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request()).unwrap();
    lake.write_ahdm_evidence_inventory("2026-06-10T00:00:00Z")
        .unwrap();
    lake.write_ahdm_pine_artifacts("2026-06-10T00:00:00Z")
        .unwrap();
    let run_dir = lake
        .run_ahdm_native_backtest(
            "run-1",
            "polygon-BTCUSD-1D-raw",
            "20260101-fixture",
            backtest_config(),
            1.0,
            "2026-06-10T00:00:00Z",
        )
        .unwrap();
    let readiness_path = lake
        .write_ahdm_paper_trading_readiness("2026-06-10T00:00:00Z")
        .unwrap();

    assert_eq!(
        readiness_path.strip_prefix(temp.path()).unwrap(),
        std::path::Path::new(
            ".archon/trading-lab/strategies/AHDM-v1/readiness/paper-trading-readiness.md"
        )
    );
    assert_eq!(
        run_dir
            .join("adversarial-review.md")
            .strip_prefix(temp.path())
            .unwrap(),
        std::path::Path::new(
            ".archon/trading-lab/strategies/AHDM-v1/backtests/run-1/adversarial-review.md"
        )
    );

    let readiness = std::fs::read_to_string(readiness_path).unwrap();
    assert!(readiness.contains("paper_trading_ready: `false`"));
    assert!(readiness.contains("promotion_eligible: `false`"));
    assert!(readiness.contains("live_trading_enabled: `false`"));
    assert!(readiness.contains("## Required Gates"));
    assert!(readiness.contains("## Done-Definition Checks"));
    assert!(readiness.contains("scope: checked"));
    assert!(readiness.contains("focused tests: checked"));
    assert!(readiness.contains("KB evidence"));
    assert!(readiness.contains("Data/provider/coverage"));
    assert!(readiness.contains("Overfitting"));
    assert!(readiness.contains("Slippage/execution"));
    assert!(readiness.contains("fail_closed_behavior"));
    assert!(readiness.contains("owner"));
    assert!(readiness.contains("created_at"));

    let adversarial = std::fs::read_to_string(run_dir.join("adversarial-review.md")).unwrap();
    assert!(adversarial.contains("# AHDM-v1 Adversarial Review"));
    assert!(adversarial.contains("no high-probability claim"));
}

#[test]
fn stores_snapshot_artifact_with_freshness_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let path = lake
        .persist_snapshot(
            crate::data_lake::CurrentSnapshot {
                provider: "tradingview".into(),
                canonical_instrument: "ES".into(),
                provider_symbol: "CME_MINI:ES1!".into(),
                captured_at_unix_seconds: 1_000,
                payload: serde_json::json!({"price": 5000.0}),
            },
            1_301,
        )
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        path,
        temp.path()
            .join(".archon/trading-lab/data/snapshots/tradingview/ES.json")
    );
    assert!(text.contains("Stale"));
    assert!(text.contains("captured_at_unix_seconds"));
}

#[test]
fn validation_summary_counts_duplicates_bad_ohlc_and_volume() {
    let mut bars = vec![
        bar("2026-01-01T00:00:00Z", 10.0),
        bar("2026-01-01T00:00:00Z", 10.0),
    ];
    bars[1].high = 5.0;
    bars[1].volume = -1.0;
    let report = validation_report(&request().metadata, &bars, "now".into());
    assert_eq!(report.summary.duplicate_timestamp_count, 1);
    assert_eq!(report.summary.bad_ohlc_count, 1);
    assert_eq!(report.summary.missing_volume_count, 1);
    assert_eq!(report.status, ValidationStatus::Failed);
    assert!(
        report
            .checks
            .iter()
            .any(|check| check.id == "ohlcv.duplicate_timestamps")
    );
    assert!(!report.production_eligible);
}

#[test]
fn d44_constant_volume_fails_validation_and_registry_reconciles_degraded() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = request();
    for bar in &mut request.bars {
        bar.volume = 1.0;
    }

    let record = lake.store_ohlcv(request).unwrap();
    assert_eq!(record.status, DatasetStatus::Degraded);
    assert!(!record.production_eligible);

    let report: ValidationReport = read_json(&temp.path().join(&record.validation_path)).unwrap();
    assert_eq!(report.status, ValidationStatus::Failed);
    assert!(!report.production_eligible);
    assert!(
        report.checks.iter().any(|check| {
            check.id == "ohlcv.volume" && check.status == ValidationStatus::Failed
        })
    );

    let registry: PersistentDatasetRegistry = read_json(&lake.registry_path()).unwrap();
    let stored = registry
        .datasets
        .get(&registry_key(&record.dataset_id, &record.version))
        .unwrap();
    assert_eq!(stored.status, DatasetStatus::Degraded);
    assert!(!stored.production_eligible);

    let metadata: DatasetMetadata = read_json(&temp.path().join(&record.metadata_path)).unwrap();
    assert_eq!(metadata.quality_status, "degraded");
    assert!(!metadata.production_eligible);
}

#[test]
fn validation_report_fails_closed_for_native_gate_invariants() {
    let mut metadata = request().metadata;
    metadata.native_interval = false;
    metadata.production_eligible = true;
    metadata.gaps.missing_bars = 1;
    let mut bars = vec![bar("2026-01-01T00:00:00Z", 10.0)];
    bars.push(bar("2026-01-01 00:00:00", 10.0));
    bars.push(bar("2026-01-01T00:00:00Z", 10.0));
    bars[2].open = f64::NAN;
    bars[2].volume = 0.0;
    metadata.coverage.observed_bars = bars.len() as u64;

    let report = validation_report(&metadata, &bars, "now".into());
    assert_eq!(report.status, ValidationStatus::Failed);
    assert!(!report.native_interval);
    assert!(!report.production_eligible);
    for required in [
        "metadata.native_interval",
        "ohlcv.rfc3339_timestamps",
        "ohlcv.duplicate_timestamps",
        "ohlcv.ohlc_sanity",
        "ohlcv.volume",
        "ohlcv.gaps",
        "ohlcv.valid_bars",
    ] {
        assert!(
            report
                .checks
                .iter()
                .any(|check| check.id == required && check.status == ValidationStatus::Failed),
            "missing failed check {required}"
        );
    }
}

#[test]
fn tampered_metadata_is_rejected_before_revalidation() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    assert_eq!(record.status, DatasetStatus::Healthy);
    assert!(record.production_eligible);
    let metadata_path = temp.path().join(&record.metadata_path);
    let mut metadata: DatasetMetadata = read_json(&metadata_path).unwrap();
    metadata.provider.clear();
    write_json(&metadata_path, &metadata).unwrap();
    let result = lake.validate_ohlcv("polygon-BTCUSD-1D-raw", "20260101-fixture", "now".into());
    assert!(matches!(
        result,
        Err(DataStoreError::IncompleteArtifactContract(message))
            if message.contains("checksum chain mismatch")
    ));
    let report: ValidationReport = read_json(&temp.path().join(&record.validation_path)).unwrap();
    assert_eq!(report.status, ValidationStatus::Passed);
    assert!(report.production_eligible);

    let registry: PersistentDatasetRegistry = read_json(&lake.registry_path()).unwrap();
    let stored = registry
        .datasets
        .get(&registry_key(&record.dataset_id, &record.version))
        .unwrap();
    assert_eq!(stored.status, DatasetStatus::Healthy);
    assert!(stored.production_eligible);

    let metadata: DatasetMetadata = read_json(&metadata_path).unwrap();
    assert!(metadata.provider.is_empty());
}

#[test]
fn backtest_gate_refuses_non_native_dataset_without_diagnostic_override() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = request();
    request.metadata.native_interval = false;
    request.metadata.production_eligible = false;
    request.metadata.quality_status = "degraded".into();
    lake.store_ohlcv(request).unwrap();

    let result = lake.backtest_data_gate("polygon-BTCUSD-1D-raw", "20260101-fixture", false);
    assert!(matches!(result, Err(DataStoreError::InvalidMetadata(_))));
}

#[test]
fn diagnostic_backtest_gate_reports_overridden_dataset_issues() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut request = request();
    request.metadata.native_interval = false;
    request.metadata.production_eligible = false;
    request.metadata.quality_status = "degraded".into();
    lake.store_ohlcv(request).unwrap();

    let report = lake
        .backtest_data_gate("polygon-BTCUSD-1D-raw", "20260101-fixture", true)
        .unwrap();
    assert!(report.diagnostic);
    assert!(!report.promotion_eligible);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.contains("provider-native") || issue.contains("native interval"))
    );
    assert_eq!(report.overridden_issues, report.issues);
}

#[test]
fn backtest_gate_refuses_checksum_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let metadata_path = temp.path().join(&record.metadata_path);
    let mut metadata: DatasetMetadata = read_json(&metadata_path).unwrap();
    metadata.checksum = "wrong-checksum".into();
    write_json(&metadata_path, &metadata).unwrap();

    let result = lake.backtest_data_gate("polygon-BTCUSD-1D-raw", "20260101-fixture", false);
    assert!(matches!(
        result,
        Err(DataStoreError::InvalidMetadata(message))
            if message.contains("backtest data gate refused dataset")
    ));
}

#[test]
fn diagnostic_backtest_gate_reports_checksum_mismatch_as_overridden_issue() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let record = lake.store_ohlcv(request()).unwrap();
    let metadata_path = temp.path().join(&record.metadata_path);
    let mut metadata: DatasetMetadata = read_json(&metadata_path).unwrap();
    metadata.checksum = "wrong-checksum".into();
    write_json(&metadata_path, &metadata).unwrap();

    let report = lake
        .backtest_data_gate("polygon-BTCUSD-1D-raw", "20260101-fixture", true)
        .unwrap();

    assert!(report.diagnostic);
    assert!(!report.promotion_eligible);
    assert!(
        report
            .issues
            .iter()
            .any(|issue| issue.contains("checksum mismatch"))
    );
    assert_eq!(report.overridden_issues, report.issues);
}

fn request() -> StoreOhlcvRequest {
    StoreOhlcvRequest {
        metadata: DatasetMetadata {
            schema_version: "archon-trading-dataset-20260101-fixture".into(),
            dataset_id: "polygon-BTCUSD-1D-raw".into(),
            version: "20260101-fixture".into(),
            canonical_instrument: "BTCUSD".into(),
            asset_class: "crypto".into(),
            provider: "polygon".into(),
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
                expected_bars: COVERAGE_MINIMUM_ROWS as u64,
                observed_bars: 0,
            },
            gaps: GapSummary {
                missing_bars: 0,
                expected_bars: COVERAGE_MINIMUM_ROWS as u64,
            },
            checksum: String::new(),
            checksums: DatasetChecksums::default(),
            paths: DatasetArtifactPaths::default(),
            source: DatasetSourceMetadata::default(),
            quality_status: "passed".into(),
            created_at: String::new(),
            optional: false,
        },
        bars: (0..COVERAGE_MINIMUM_ROWS)
            .map(|index| {
                let month = (index / 28) + 1;
                let day = (index % 28) + 1;
                bar(
                    &format!("2026-{month:02}-{day:02}T00:00:00Z"),
                    10.0 + index as f64,
                )
            })
            .collect(),
        raw_body: b"raw".to_vec(),
        raw_format: OhlcvFormat::Csv,
        raw_request: serde_json::json!({"source":"test"}),
        redacted_headers: serde_json::json!({}),
        provider_notes: "test fixture".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
    }
}
