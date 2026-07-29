use super::*;

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

    assert_eq!(report["schema"], "archon-ahdm-pine-compile-report-v1");
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
    assert!(indicator.contains("shared_rule_id="));
    assert!(indicator.contains("raw_score"));
    assert!(strategy.contains("strategy.entry"));
    assert!(strategy.contains("strategy.exit"));
    assert_eq!(report["tooling_available"], true);
    assert_eq!(report["promotion_eligible"], false);
    assert_eq!(report["pine_results"], "exploratory_only");
    assert_eq!(
        report["shared_manifest_traceability"]["native_and_pine_parity_key"],
        "AHDM-v1/shared-rule-manifest"
    );
    assert!(
        report["shared_manifest_traceability"]["indicator_sha256"]
            .as_str()
            .unwrap()
            .len()
            > 20
    );
    assert!(
        report["shared_manifest_traceability"]["strategy_sha256"]
            .as_str()
            .unwrap()
            .len()
            > 20
    );
    assert_eq!(report["required_tooling"].as_array().unwrap().len(), 6);
    let tooling_results = report["tooling_results"].as_array().unwrap();
    assert_eq!(tooling_results.len(), 8);
    for result in tooling_results {
        let invocation = result["invocation"].as_str().unwrap();
        assert!(invocation.starts_with("mcp__tradingview__pine_"));
        assert_eq!(result["capture_required"], true);
        assert_eq!(result["promotion_evidence"], false);
    }
    assert!(
        tooling_results
            .iter()
            .any(|result| result["tool"] == "mcp__tradingview__pine_compile")
    );
    assert!(
        tooling_results
            .iter()
            .any(|result| result["tool"] == "mcp__tradingview__pine_smart_compile")
    );
    assert_eq!(
        report["residual_gaps"][0]["id"],
        "GAP-AHDM-PINE-CHART-COMPILE-001"
    );
    assert_eq!(report["residual_gaps"][0]["fail_closed"], true);
    assert!(
        report["residual_gaps"][0]["captured_mcp_invocations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|invocation| invocation.as_str().unwrap() == "mcp__tradingview__pine_compile()")
    );
    assert!(
        report["residual_gaps"][0]["blocks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|block| block == "live_trading_promotion")
    );
}

#[test]
fn ahdm_native_backtest_rejects_short_production_history() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(short_production_history_request())
        .unwrap();

    let result = lake.run_ahdm_native_backtest(
        "short-history",
        "tradingview-BTCUSD-1D-raw",
        "20260101-short-history",
        backtest_config(),
        1.0,
        "2026-06-10T00:00:00Z",
    );

    assert!(matches!(
        result,
        Err(DataStoreError::InvalidMetadata(message))
            if message.contains("below required production backtest minimum")
    ));
}

fn short_production_history_request() -> StoreOhlcvRequest {
    let mut request = request();
    request.metadata.provider = "tradingview".into();
    request.metadata.provider_symbol = provider_symbol("BTCUSD", "tradingview");
    request.metadata.dataset_id = "tradingview-BTCUSD-1D-raw".into();
    request.metadata.version = "20260101-short-history".into();
    request.metadata.coverage.expected_bars = (COVERAGE_MINIMUM_ROWS * 2) as u64;
    request.metadata.gaps.expected_bars = (COVERAGE_MINIMUM_ROWS * 2) as u64;
    request.bars.truncate(COVERAGE_MINIMUM_ROWS / 2);
    request.raw_request = serde_json::json!({"source":"live provider fetch test capture"});
    request.provider_notes = "captured live provider response".into();
    request
}
