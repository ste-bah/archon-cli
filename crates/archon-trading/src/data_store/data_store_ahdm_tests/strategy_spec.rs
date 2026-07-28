use super::*;
use crate::data_store::ahdm_test_support::store_complete_trading_core_coverage;

#[test]
fn ahdm_strategy_spec_contains_required_model_contract() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    lake.store_ohlcv(request()).unwrap();
    let path = lake
        .write_ahdm_strategy_spec("2026-06-10T00:00:00Z")
        .unwrap();
    let spec: serde_json::Value = read_json(&path).unwrap();

    assert_eq!(spec["schema"], "archon-ahdm-strategy-spec-v1");
    assert!(spec.get("schema_version").is_none());
    assert_eq!(spec["confidence_scoring"]["type"], "score_not_probability");
    assert_eq!(spec["confidence_scoring"]["sum_weights"], 100);
    assert_eq!(spec["confidence_scoring"]["no_trade_below"], 0.55);
    assert_eq!(spec["confidence_scoring"]["paper_consideration_min"], 0.70);
    assert_eq!(
        spec["required_datasets"][0]["dataset_id"],
        "polygon-BTCUSD-1D-raw"
    );
    assert_entry_model_contract(&spec);
    assert_eq!(spec["coverage_universe"]["required_cells"], 30);
    assert_eq!(spec["coverage_universe"]["available_cells"], 1);
    assert_eq!(spec["coverage_universe"]["promotion_eligible"], false);
    assert_eq!(spec["dataset_coverage_gate"]["promotion_eligible"], false);
    assert_eq!(
        spec["coverage_universe"]["promotion_eligible"],
        spec["dataset_coverage_gate"]["promotion_eligible"]
    );
    assert_eq!(
        spec["dataset_coverage_gate"]["dataset_refs"]
            .as_array()
            .unwrap()
            .len(),
        spec["required_datasets"].as_array().unwrap().len()
    );
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
fn ahdm_strategy_spec_mirrors_degraded_registry_refs_and_fails_closed() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    let mut degraded_ref = request();
    degraded_ref.metadata.provider = "tradingview".into();
    degraded_ref.metadata.canonical_instrument = "ES".into();
    degraded_ref.metadata.provider_symbol = "ES1!".into();
    degraded_ref.metadata.symbol_map = BTreeMap::from([("ES".into(), "ES1!".into())]);
    degraded_ref.metadata.dataset_id = "tradingview-ES-1W-raw-tdl080".into();
    degraded_ref.metadata.version = "20260721-tdl080-required-universe".into();
    degraded_ref.metadata.timeframe = "1W".into();
    degraded_ref.metadata.production_eligible = false;
    lake.store_ohlcv(degraded_ref).unwrap();

    let path = lake
        .write_ahdm_strategy_spec("2026-06-10T00:00:00Z")
        .unwrap();
    let spec: serde_json::Value = read_json(&path).unwrap();
    let first_ref = &spec["required_datasets"][0];

    assert_eq!(first_ref["dataset_id"], "tradingview-ES-1W-raw-tdl080");
    assert_eq!(first_ref["version"], "20260721-tdl080-required-universe");
    assert_eq!(first_ref["native_interval"], true);
    assert_eq!(first_ref["production_eligible"], false);
    assert_eq!(first_ref["status"], "Degraded");
    assert_eq!(spec["coverage_universe"]["available_cells"], 0);
    assert_eq!(spec["coverage_universe"]["required_cells"], 30);
    assert_eq!(spec["coverage_universe"]["promotion_eligible"], false);
    assert_eq!(spec["dataset_coverage_gate"]["promotion_eligible"], false);
    assert_eq!(
        spec["dataset_coverage_gate"]["unavailable_refs"][0]["registry_ref"],
        "tradingview-ES-1W-raw-tdl080:20260721-tdl080-required-universe"
    );
    assert!(spec["residual_gaps"].as_array().unwrap().iter().any(|gap| {
        gap["id"] == "GAP-AHDM-DATA-COVERAGE-001"
            && gap["fail_closed_behavior"]
                .as_str()
                .unwrap()
                .contains("Strategy validation and promotion remain blocked")
    }));
}

#[test]
fn ahdm_strategy_spec_counts_only_promotable_required_cells() {
    let temp = tempfile::tempdir().unwrap();
    let lake = TradingDataLake::new(temp.path());
    store_complete_trading_core_coverage(&lake);

    let path = lake
        .write_ahdm_strategy_spec("2026-06-10T00:00:00Z")
        .unwrap();
    let spec: serde_json::Value = read_json(&path).unwrap();

    assert_eq!(spec["coverage_universe"]["available_cells"], 30);
    assert_eq!(spec["coverage_universe"]["promotion_eligible"], true);
    assert!(
        spec["dataset_coverage_gate"]["unavailable_refs"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

fn assert_entry_model_contract(spec: &serde_json::Value) {
    let entry_models = spec["entry_models"].as_array().unwrap();
    assert_eq!(entry_models.len(), 3);
    for model in entry_models {
        for field in [
            "evidence_requirements",
            "fail_closed_no_trade",
            "invalidation",
            "stop",
            "tp1",
            "tp2",
            "tp3",
            "filters",
            "sizing",
        ] {
            assert!(model.get(field).is_some(), "missing {field} in {model:?}");
        }
        assert_eq!(model["fail_closed_no_trade"], true);
        assert_eq!(model["sizing"]["risk_fraction"], 0.005);
        assert_eq!(model["sizing"]["max_fraction"], 0.01);
        assert_eq!(model["sizing"]["invalid_inputs"], "no_trade_fail_closed");
        assert!(model["evidence_requirements"].as_array().unwrap().len() >= 3);
        assert!(model["filters"].as_array().unwrap().len() >= 3);
    }
}
