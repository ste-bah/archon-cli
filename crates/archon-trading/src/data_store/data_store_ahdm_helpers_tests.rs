use super::*;
pub(super) fn bar(timestamp: &str, close: f64) -> OhlcvBar {
    OhlcvBar {
        timestamp: timestamp.into(),
        open: close,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: close * 1_000.0,
    }
}

pub(super) fn backtest_config() -> BacktestConfig {
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
