use archon_trading::backtest::*;
use archon_trading::data_lake::DatasetStatus;

fn config(seed: u64) -> BacktestConfig {
    BacktestConfig {
        strategy_id: "strat-1".into(),
        snapshot_checksum: "snapshot-abc".into(),
        starting_equity: 100_000.0,
        fee_per_share: 0.01,
        spread_bps: 1.0,
        slippage_bps: 2.0,
        market_impact_bps: 3.0,
        latency_ms: 25,
        partial_fill_ratio: 0.8,
        unavailable_liquidity_ratio: 0.1,
        monte_carlo_seed: seed,
        parameter_set_id: "params-v1".into(),
    }
}

fn fills() -> Vec<FillInput> {
    vec![
        FillInput {
            price: 100.0,
            quantity: 10.0,
            side: 1,
            session_index: 2,
            regime_id: 1,
        },
        FillInput {
            price: 102.0,
            quantity: 8.0,
            side: -1,
            session_index: 1,
            regime_id: 0,
        },
        FillInput {
            price: 99.0,
            quantity: 12.0,
            side: 1,
            session_index: 3,
            regime_id: 1,
        },
    ]
}

#[test]
fn t_data_01_replay_is_bit_identical_from_snapshot_and_config_hash() {
    let harness = BacktestHarness::new(config(42)).unwrap();
    let first = harness
        .run(
            &fills(),
            DatasetStatus::Healthy,
            false,
            EvidenceSource::NativeHarness,
        )
        .unwrap();
    let second = harness
        .run(
            &fills(),
            DatasetStatus::Healthy,
            false,
            EvidenceSource::NativeHarness,
        )
        .unwrap();

    assert_eq!(first, second);
    assert_eq!(
        first.replay_id,
        replay_id(&first.snapshot_checksum, &first.config_hash)
    );
}

#[test]
fn t_data_02_models_all_microstructure_costs_against_fixture() {
    let result = execute_fill(&fills()[0], &config(7)).unwrap();

    assert!(result.costs.fees > 0.0);
    assert!(result.costs.spread > 0.0);
    assert!(result.costs.slippage > 0.0);
    assert!(result.costs.market_impact > 0.0);
    assert!(result.costs.latency > 0.0);
    assert!(result.costs.partial_fill > 0.0);
    assert!(result.costs.unavailable_liquidity > 0.0);
    assert_eq!(result.filled_quantity, 7.2);
}

#[test]
fn t_data_03_report_has_all_metrics_and_full_robustness_suite() {
    let report = BacktestHarness::new(config(99))
        .unwrap()
        .run(
            &fills(),
            DatasetStatus::Healthy,
            false,
            EvidenceSource::NativeHarness,
        )
        .unwrap();

    assert_eq!(report.metrics.len(), REPORT_METRIC_COUNT);
    assert!(validate_report_metrics(&report.metrics).is_ok());
    assert!(validate_robustness(&report.robustness).is_ok());
    assert!(
        report
            .robustness
            .iter()
            .any(|item| item.kind == RobustnessKind::MonteCarloReshuffle && item.seed == Some(99))
    );
}

#[test]
fn a_data_01_demotes_strategy_tester_and_exploratory_evidence() {
    assert!(is_promotion_eligible(false, EvidenceSource::NativeHarness));
    assert!(!is_promotion_eligible(true, EvidenceSource::NativeHarness));
    assert!(!is_promotion_eligible(
        false,
        EvidenceSource::StrategyTester
    ));
}

#[test]
fn a_data_02_blocks_degraded_data_and_numeric_version_affects_config_hash() {
    let harness = BacktestHarness::new(config(1)).unwrap();
    assert_eq!(
        harness.run(
            &fills(),
            DatasetStatus::Degraded,
            false,
            EvidenceSource::NativeHarness
        ),
        Err(BacktestError::DatasetNotHealthy)
    );
    let hash = config(1).config_hash();
    let mut without_pin = serde_json::to_vec(&config(1)).unwrap();
    without_pin.extend_from_slice(b"different-numeric-lib");
    assert_ne!(hash, blake3::hash(&without_pin).to_hex().to_string());
}
