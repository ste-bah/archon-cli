use crate::backtest::{BacktestConfig, EvidenceSource};
use crate::custom_strategy::{CustomOhlcvStrategy, CustomStrategyError, custom_entry_exit_pairs};
use crate::ohlcv::{OhlcvBacktestRequest, OhlcvBacktestRule, OhlcvBar};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhlcvTrade {
    pub entry_index: usize,
    pub exit_index: usize,
    pub entry_timestamp: String,
    pub exit_timestamp: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub quantity: f64,
    pub gross_pnl: f64,
    pub cost_total: f64,
    pub net_pnl: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OhlcvBacktestReport {
    pub strategy_id: String,
    pub dataset_id: String,
    pub dataset_version: String,
    pub dataset_checksum: String,
    pub config_hash: String,
    pub rule: String,
    pub exploratory: bool,
    pub source: EvidenceSource,
    pub promotion_eligible: bool,
    pub metrics: BTreeMap<String, f64>,
    pub trades: Vec<OhlcvTrade>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandleBacktestError {
    InvalidConfig(&'static str),
    NotEnoughBars,
    CustomStrategy(CustomStrategyError),
}

pub fn run_ohlcv_backtest(
    config: &BacktestConfig,
    request: &OhlcvBacktestRequest,
    bars: &[OhlcvBar],
) -> Result<OhlcvBacktestReport, CandleBacktestError> {
    validate_request(request, bars)?;
    let trades = match request.rule {
        OhlcvBacktestRule::CloseMomentum => close_momentum_trades(config, request, bars),
        OhlcvBacktestRule::SmaCross => sma_cross_trades(config, request, bars),
    };
    let metrics = metrics(config.starting_equity, &trades);
    Ok(OhlcvBacktestReport {
        strategy_id: config.strategy_id.clone(),
        dataset_id: request.dataset.dataset_id.clone(),
        dataset_version: request.dataset.version.clone(),
        dataset_checksum: request.dataset.checksum.clone(),
        config_hash: config.config_hash(),
        rule: format!("{:?}", request.rule),
        exploratory: request.exploratory,
        source: request.source,
        promotion_eligible: !request.exploratory && request.source == EvidenceSource::NativeHarness,
        metrics,
        trades,
    })
}

fn validate_request(
    request: &OhlcvBacktestRequest,
    bars: &[OhlcvBar],
) -> Result<(), CandleBacktestError> {
    if bars.len() < 2 {
        return Err(CandleBacktestError::NotEnoughBars);
    }
    if !request.quantity.is_finite() || request.quantity <= 0.0 {
        return Err(CandleBacktestError::InvalidConfig("quantity"));
    }
    if request.rule == OhlcvBacktestRule::SmaCross && request.fast_len >= request.slow_len {
        return Err(CandleBacktestError::InvalidConfig("sma_lengths"));
    }
    Ok(())
}

fn close_momentum_trades(
    config: &BacktestConfig,
    request: &OhlcvBacktestRequest,
    bars: &[OhlcvBar],
) -> Vec<OhlcvTrade> {
    let mut trades = Vec::new();
    let mut entry = None;
    for index in 1..bars.len() {
        if bars[index].close > bars[index - 1].close && entry.is_none() {
            entry = Some(index);
        } else if bars[index].close < bars[index - 1].close
            && let Some(entry_index) = entry.take()
        {
            trades.push(trade(config, request.quantity, entry_index, index, bars));
        }
    }
    close_open_trade(config, request.quantity, entry, bars, &mut trades);
    trades
}

fn sma_cross_trades(
    config: &BacktestConfig,
    request: &OhlcvBacktestRequest,
    bars: &[OhlcvBar],
) -> Vec<OhlcvTrade> {
    let mut trades = Vec::new();
    let mut entry = None;
    for index in request.slow_len..bars.len() {
        let fast = sma(bars, index, request.fast_len);
        let slow = sma(bars, index, request.slow_len);
        if fast > slow && entry.is_none() {
            entry = Some(index);
        } else if fast <= slow
            && let Some(entry_index) = entry.take()
        {
            trades.push(trade(config, request.quantity, entry_index, index, bars));
        }
    }
    close_open_trade(config, request.quantity, entry, bars, &mut trades);
    trades
}

fn close_open_trade(
    config: &BacktestConfig,
    quantity: f64,
    entry: Option<usize>,
    bars: &[OhlcvBar],
    trades: &mut Vec<OhlcvTrade>,
) {
    if let Some(entry_index) = entry {
        trades.push(trade(config, quantity, entry_index, bars.len() - 1, bars));
    }
}

pub fn run_custom_ohlcv_backtest(
    config: &BacktestConfig,
    request: &OhlcvBacktestRequest,
    bars: &[OhlcvBar],
    strategy: &CustomOhlcvStrategy,
) -> Result<OhlcvBacktestReport, CandleBacktestError> {
    validate_request(request, bars)?;
    let trades = custom_entry_exit_pairs(strategy, bars)
        .map_err(CandleBacktestError::CustomStrategy)?
        .into_iter()
        .map(|(entry, exit)| trade(config, request.quantity, entry, exit, bars))
        .collect::<Vec<_>>();
    let metrics = metrics(config.starting_equity, &trades);
    Ok(OhlcvBacktestReport {
        strategy_id: config.strategy_id.clone(),
        dataset_id: request.dataset.dataset_id.clone(),
        dataset_version: request.dataset.version.clone(),
        dataset_checksum: request.dataset.checksum.clone(),
        config_hash: config.config_hash(),
        rule: custom_rule_name(strategy),
        exploratory: request.exploratory,
        source: request.source,
        promotion_eligible: !request.exploratory && request.source == EvidenceSource::NativeHarness,
        metrics,
        trades,
    })
}

fn trade(
    config: &BacktestConfig,
    quantity: f64,
    entry_index: usize,
    exit_index: usize,
    bars: &[OhlcvBar],
) -> OhlcvTrade {
    let entry = &bars[entry_index];
    let exit = &bars[exit_index];
    let gross_pnl = (exit.close - entry.close) * quantity;
    let cost_total = roundtrip_cost(config, entry.close, exit.close, quantity);
    OhlcvTrade {
        entry_index,
        exit_index,
        entry_timestamp: entry.timestamp.clone(),
        exit_timestamp: exit.timestamp.clone(),
        entry_price: entry.close,
        exit_price: exit.close,
        quantity,
        gross_pnl,
        cost_total,
        net_pnl: gross_pnl - cost_total,
    }
}

fn roundtrip_cost(config: &BacktestConfig, entry: f64, exit: f64, quantity: f64) -> f64 {
    let notional = (entry + exit) * quantity;
    let bps = config.spread_bps + config.slippage_bps + config.market_impact_bps;
    quantity * config.fee_per_share * 2.0 + notional * bps / 10_000.0
}

fn sma(bars: &[OhlcvBar], end: usize, len: usize) -> f64 {
    let start = end + 1 - len;
    bars[start..=end].iter().map(|bar| bar.close).sum::<f64>() / len as f64
}

fn metrics(starting_equity: f64, trades: &[OhlcvTrade]) -> BTreeMap<String, f64> {
    let pnl = trades.iter().map(|trade| trade.net_pnl).collect::<Vec<_>>();
    BTreeMap::from([
        ("net_profit".into(), pnl.iter().sum()),
        ("max_drawdown".into(), max_drawdown(starting_equity, &pnl)),
        ("win_rate".into(), win_rate(&pnl)),
        ("trade_count".into(), trades.len() as f64),
        ("avg_trade".into(), average(&pnl)),
        (
            "cost_total".into(),
            trades.iter().map(|trade| trade.cost_total).sum(),
        ),
    ])
}

fn max_drawdown(starting_equity: f64, pnl: &[f64]) -> f64 {
    let mut equity = starting_equity;
    let mut peak = starting_equity;
    let mut max_dd = 0.0;
    for value in pnl {
        equity += value;
        peak = peak.max(equity);
        max_dd = f64::max(max_dd, (peak - equity) / peak.max(1.0));
    }
    max_dd
}

fn average(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn win_rate(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().filter(|value| **value > 0.0).count() as f64 / values.len() as f64
    }
}

fn custom_rule_name(strategy: &CustomOhlcvStrategy) -> String {
    strategy
        .name
        .as_ref()
        .map(|name| format!("custom:{name}"))
        .unwrap_or_else(|| "custom".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data_lake::DatasetStatus;
    use crate::ohlcv::OhlcvDatasetRef;

    #[test]
    fn close_momentum_creates_replayable_trade() {
        let report = run_ohlcv_backtest(&config(), &request(), &bars()).unwrap();
        assert_eq!(report.trades.len(), 1);
        assert_eq!(report.dataset_id, "btc-1d");
        assert!(report.metrics["net_profit"].is_finite());
    }

    fn request() -> OhlcvBacktestRequest {
        OhlcvBacktestRequest {
            dataset: OhlcvDatasetRef {
                dataset_id: "btc-1d".into(),
                version: "v1".into(),
                checksum: "abc".into(),
                status: DatasetStatus::Healthy,
            },
            rule: OhlcvBacktestRule::CloseMomentum,
            quantity: 1.0,
            exploratory: false,
            source: EvidenceSource::NativeHarness,
            fast_len: 3,
            slow_len: 5,
        }
    }

    fn config() -> BacktestConfig {
        BacktestConfig {
            strategy_id: "s1".into(),
            snapshot_checksum: "abc".into(),
            starting_equity: 10_000.0,
            fee_per_share: 0.0,
            spread_bps: 0.0,
            slippage_bps: 0.0,
            market_impact_bps: 0.0,
            latency_ms: 0,
            partial_fill_ratio: 1.0,
            unavailable_liquidity_ratio: 0.0,
            monte_carlo_seed: 7,
            parameter_set_id: "p1".into(),
        }
    }

    fn bars() -> Vec<OhlcvBar> {
        vec![
            bar("1", 10.0),
            bar("2", 11.0),
            bar("3", 12.0),
            bar("4", 11.0),
        ]
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
}
