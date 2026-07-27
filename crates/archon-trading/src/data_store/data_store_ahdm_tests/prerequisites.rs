use super::*;

#[test]
fn ahdm_evidence_inventory_records_verified_prerequisite_gates() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    for instrument in ["ES", "NQ", "SPY", "QQQ", "BTCUSDT", "ETHUSDT"] {
        for timeframe in ["1W", "1D", "240", "60", "15"] {
            let mut request = request();
            request.metadata.dataset_id = format!("polygon-{instrument}-{timeframe}-raw");
            request.metadata.canonical_instrument = instrument.into();
            request.metadata.provider_symbol = instrument.into();
            request.metadata.timeframe = timeframe.into();
            request.metadata.symbol_map = BTreeMap::from([(instrument.into(), instrument.into())]);
            lake.store_ohlcv(request).unwrap();
        }
    }

    lake.write_ahdm_evidence_inventory_with_backtest_gate("2026-06-10T00:00:00Z", true)
        .unwrap();
    let citations: serde_json::Value =
        read_json(&lake.ahdm_strategy_root().join("evidence/citations.json")).unwrap();

    assert_eq!(citations["coverage_gaps"].as_array().unwrap().len(), 0);
    assert_eq!(
        citations["inventory_gate"]["required_prerequisites"]["data_coverage_gate_passed"],
        true
    );
    assert_eq!(
        citations["inventory_gate"]["required_prerequisites"]["native_backtest_gate_passed"],
        true
    );
    assert_eq!(citations["promotion_allowed"], false);

    let inventory = std::fs::read_to_string(
        lake.ahdm_strategy_root()
            .join("evidence/kb-rule-inventory.md"),
    )
    .unwrap();
    assert!(inventory.contains("none recorded by coverage matrix"));
    assert!(
        !inventory.contains("project artifact generated without coverage-matrix prerequisites")
    );
    assert!(!inventory.contains("GAP-AHDM-DATA-001"));
    assert!(inventory.contains("GAP-AHDM-KB-HYPOTHESIS-001"));
}
