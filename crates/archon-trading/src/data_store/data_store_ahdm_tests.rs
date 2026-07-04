use super::*;
use crate::data_lake::{CoverageWindow, DataType, GapSummary};
use crate::data_store::ahdm_test_support::ahdm_position_size;

#[test]
fn ahdm_evidence_inventory_and_citations_fail_closed_for_gaps() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.write_ahdm_evidence_inventory("2026-06-10T00:00:00Z")
        .unwrap();
    let citations: serde_json::Value =
        read_json(&lake.ahdm_strategy_root().join("evidence/citations.json")).unwrap();
    assert_eq!(
        citations["promotion_policy"]["hypotheses_barred_from_promotion_until_cited"],
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

    let inventory = std::fs::read_to_string(
        lake.ahdm_strategy_root()
            .join("evidence/kb-rule-inventory.md"),
    )
    .unwrap();
    assert!(!inventory.contains("trading-elliott-wave"));
}

#[test]
fn ahdm_strategy_spec_contains_required_model_contract() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request()).unwrap();
    let path = lake
        .write_ahdm_strategy_spec("2026-06-10T00:00:00Z")
        .unwrap();
    let spec: serde_json::Value = read_json(&path).unwrap();

    assert_eq!(spec["schema_version"], "archon-ahdm-strategy-spec-v1");
    assert_eq!(spec["confidence_scoring"]["type"], "score_not_probability");
    assert_eq!(spec["confidence_scoring"]["sum_weights"], 100);
    assert_eq!(spec["confidence_scoring"]["no_trade_below"], 0.55);
    assert_eq!(spec["confidence_scoring"]["paper_consideration_min"], 0.70);
    assert_eq!(
        spec["required_datasets"][0]["dataset_id"],
        "manual-BTCUSD-1D-raw"
    );
    assert_eq!(spec["entry_models"].as_array().unwrap().len(), 3);
    assert!(
        spec["promotion_gates"]["live"]
            .as_str()
            .unwrap()
            .contains("out_of_scope")
    );

    let rules = spec["daily_bias_formula"].as_array().unwrap();
    let weight_sum = rules
        .iter()
        .map(|rule| rule["weight"].as_u64().unwrap())
        .sum::<u64>();
    assert_eq!(weight_sum, 100);
    assert!(rules.iter().all(|rule| {
        rule["status"] == "cited" && rule["citation"].is_object()
            || rule["status"] == "hypothesis" && rule["promotion_allowed"] == false
    }));
}

#[test]
fn ahdm_position_sizing_is_capped_and_invalid_inputs_no_trade() {
    assert_eq!(ahdm_position_size(10_000.0, 100.0, 95.0), Some(1.0));
    assert_eq!(ahdm_position_size(10_000.0, 100.0, 99.9), Some(1.0));
    assert_eq!(ahdm_position_size(10_000.0, 100.0, 100.0), None);
    assert_eq!(ahdm_position_size(10_000.0, 0.0, 95.0), None);
}

#[test]
fn ahdm_pine_artifacts_are_exploratory_and_manifest_traceable() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.write_ahdm_pine_artifacts("2026-06-10T00:00:00Z")
        .unwrap();
    let indicator = std::fs::read_to_string(
        lake.ahdm_strategy_root()
            .join("pine/AHDM-v1-indicator.pine"),
    )
    .unwrap();
    let strategy =
        std::fs::read_to_string(lake.ahdm_strategy_root().join("pine/AHDM-v1-strategy.pine"))
            .unwrap();
    let report: serde_json::Value =
        read_json(&lake.ahdm_strategy_root().join("pine/compile-report.json")).unwrap();

    assert!(indicator.contains("indicator(\"AHDM-v1 indicator\""));
    assert!(strategy.contains("strategy(\"AHDM-v1 strategy\""));
    for required in [
        "confidence_score",
        "no_trade_state",
        "entry_zone",
        "stop",
        "tp1",
        "tp2",
        "tp3",
        "sizing_hint",
    ] {
        assert!(indicator.contains(required));
        assert!(strategy.contains(required));
    }
    assert_eq!(report["tooling_available"], false);
    assert_eq!(report["promotion_eligible"], false);
    assert_eq!(report["pine_results"], "exploratory_only");
}

#[test]
fn ahdm_native_backtest_writes_replayable_manifest_parity_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request()).unwrap();
    let dir = lake
        .run_ahdm_native_backtest(
            "run-1",
            "manual-BTCUSD-1D-raw",
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
    assert_eq!(config["dataset"]["dataset_id"], "manual-BTCUSD-1D-raw");
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
            "manual-BTCUSD-1D-raw",
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
    assert!(readiness.contains("status: `failed`"));
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

fn request() -> StoreOhlcvRequest {
    StoreOhlcvRequest {
        metadata: DatasetMetadata {
            schema_version: "archon-trading-dataset-20260101-fixture".into(),
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

fn bar(timestamp: &str, close: f64) -> OhlcvBar {
    OhlcvBar {
        timestamp: timestamp.into(),
        open: close,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: 1.0,
    }
}

fn backtest_config() -> BacktestConfig {
    BacktestConfig {
        strategy_id: "AHDM-20260101-fixture".into(),
        snapshot_checksum: "snapshot".into(),
        starting_equity: 10_000.0,
        fee_per_share: 0.01,
        spread_bps: 1.0,
        slippage_bps: 2.0,
        market_impact_bps: 0.5,
        latency_ms: 0,
        partial_fill_ratio: 0.0,
        unavailable_liquidity_ratio: 0.0,
        monte_carlo_seed: 42,
        parameter_set_id: "fixed".into(),
    }
}
