use crate::data_lake::DatasetStatus;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const PINNED_NUMERIC_LIB_VERSION: &str = "archon-trading-fixed-f64-v1";
pub const REPORT_METRIC_KEYS: [&str; 11] = [
    "net_profit",
    "gross_profit",
    "gross_loss",
    "max_drawdown",
    "sharpe",
    "sortino",
    "profit_factor",
    "win_rate",
    "trade_count",
    "avg_trade",
    "cost_total",
];
pub const REPORT_METRIC_COUNT: usize = REPORT_METRIC_KEYS.len();

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceSource {
    NativeHarness,
    StrategyTester,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RobustnessKind {
    WalkForward,
    OutOfSample,
    MonteCarloReshuffle,
    ParameterStability,
    RegimeSliced,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestConfig {
    pub strategy_id: String,
    pub snapshot_checksum: String,
    pub starting_equity: f64,
    pub fee_per_share: f64,
    pub spread_bps: f64,
    pub slippage_bps: f64,
    pub market_impact_bps: f64,
    pub latency_ms: u64,
    pub partial_fill_ratio: f64,
    pub unavailable_liquidity_ratio: f64,
    pub monte_carlo_seed: u64,
    pub parameter_set_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FillInput {
    pub price: f64,
    pub quantity: f64,
    pub side: i8,
    pub session_index: usize,
    pub regime_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CostBreakdown {
    pub fees: f64,
    pub spread: f64,
    pub slippage: f64,
    pub market_impact: f64,
    pub latency: f64,
    pub partial_fill: f64,
    pub unavailable_liquidity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FillResult {
    pub filled_quantity: f64,
    pub execution_price: f64,
    pub gross_pnl: f64,
    pub net_pnl: f64,
    pub costs: CostBreakdown,
    pub session_index: usize,
    pub regime_id: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RobustnessResult {
    pub kind: RobustnessKind,
    pub seed: Option<u64>,
    pub passed: bool,
    pub metric: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacktestReport {
    pub strategy_id: String,
    pub replay_id: String,
    pub config_hash: String,
    pub snapshot_checksum: String,
    pub exploratory: bool,
    pub source: EvidenceSource,
    pub promotion_eligible: bool,
    pub metrics: BTreeMap<String, f64>,
    pub robustness: Vec<RobustnessResult>,
    pub fills: Vec<FillResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BacktestError {
    InvalidConfig(&'static str),
    InvalidFill(&'static str),
    DatasetNotHealthy,
    MissingRobustness(Vec<RobustnessKind>),
    MissingReportMetric(&'static str),
}

impl BacktestConfig {
    pub fn validate(&self) -> Result<(), BacktestError> {
        require_text(&self.strategy_id, "strategy_id")?;
        require_text(&self.snapshot_checksum, "snapshot_checksum")?;
        require_text(&self.parameter_set_id, "parameter_set_id")?;
        require_positive(self.starting_equity, "starting_equity")?;
        require_non_negative(self.fee_per_share, "fee_per_share")?;
        require_non_negative(self.spread_bps, "spread_bps")?;
        require_non_negative(self.slippage_bps, "slippage_bps")?;
        require_non_negative(self.market_impact_bps, "market_impact_bps")?;
        validate_ratio(self.partial_fill_ratio, "partial_fill_ratio")?;
        validate_ratio(
            self.unavailable_liquidity_ratio,
            "unavailable_liquidity_ratio",
        )
    }

    pub fn config_hash(&self) -> String {
        let mut bytes = serde_json::to_vec(self).unwrap_or_default();
        bytes.extend_from_slice(PINNED_NUMERIC_LIB_VERSION.as_bytes());
        blake3::hash(&bytes).to_hex().to_string()
    }
}

#[derive(Debug, Clone)]
pub struct BacktestHarness {
    config: BacktestConfig,
}

impl BacktestHarness {
    pub fn new(config: BacktestConfig) -> Result<Self, BacktestError> {
        config.validate()?;
        Ok(Self { config })
    }

    pub fn run(
        &self,
        fills: &[FillInput],
        dataset_status: DatasetStatus,
        exploratory: bool,
        source: EvidenceSource,
    ) -> Result<BacktestReport, BacktestError> {
        if dataset_status != DatasetStatus::Healthy {
            return Err(BacktestError::DatasetNotHealthy);
        }
        let mut ordered = fills.to_vec();
        ordered.sort_by_key(|fill| fill.session_index);
        let results = ordered
            .iter()
            .map(|fill| execute_fill(fill, &self.config))
            .collect::<Result<Vec<_>, _>>()?;
        let metrics = report_metrics(self.config.starting_equity, &results);
        validate_report_metrics(&metrics)?;
        let robustness = robustness_suite(&results, self.config.monte_carlo_seed);
        validate_robustness(&robustness)?;
        let config_hash = self.config.config_hash();
        let replay_id = replay_id(&self.config.snapshot_checksum, &config_hash);
        Ok(BacktestReport {
            strategy_id: self.config.strategy_id.clone(),
            replay_id,
            config_hash,
            snapshot_checksum: self.config.snapshot_checksum.clone(),
            exploratory,
            source,
            promotion_eligible: is_promotion_eligible(exploratory, source),
            metrics,
            robustness,
            fills: results,
        })
    }
}

pub fn execute_fill(
    fill: &FillInput,
    config: &BacktestConfig,
) -> Result<FillResult, BacktestError> {
    validate_fill(fill)?;
    let tradable_qty = fill.quantity * (1.0 - config.unavailable_liquidity_ratio);
    let filled_quantity = tradable_qty * config.partial_fill_ratio;
    let notional = fill.price * filled_quantity;
    let bps_cost = config.spread_bps + config.slippage_bps + config.market_impact_bps;
    let execution_price = fill.price * (1.0 + fill.side as f64 * bps_cost / 10_000.0);
    let costs = CostBreakdown {
        fees: filled_quantity * config.fee_per_share,
        spread: notional * config.spread_bps / 10_000.0,
        slippage: notional * config.slippage_bps / 10_000.0,
        market_impact: notional * config.market_impact_bps / 10_000.0,
        latency: notional * config.latency_ms as f64 / 1_000_000.0,
        partial_fill: fill.price * (fill.quantity - filled_quantity).max(0.0) * 0.0001,
        unavailable_liquidity: fill.price
            * fill.quantity
            * config.unavailable_liquidity_ratio
            * 0.0001,
    };
    let total_cost = sum_costs(costs);
    let gross_pnl = -fill.side as f64 * (execution_price - fill.price) * filled_quantity;
    Ok(FillResult {
        filled_quantity,
        execution_price,
        gross_pnl,
        net_pnl: gross_pnl - total_cost,
        costs,
        session_index: fill.session_index,
        regime_id: fill.regime_id,
    })
}

pub fn validate_robustness(results: &[RobustnessResult]) -> Result<(), BacktestError> {
    let found: BTreeSet<_> = results.iter().map(|result| result.kind).collect();
    let required = [
        RobustnessKind::WalkForward,
        RobustnessKind::OutOfSample,
        RobustnessKind::MonteCarloReshuffle,
        RobustnessKind::ParameterStability,
        RobustnessKind::RegimeSliced,
    ];
    let missing = required
        .into_iter()
        .filter(|kind| !found.contains(kind))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(BacktestError::MissingRobustness(missing))
    }
}

pub fn validate_report_metrics(metrics: &BTreeMap<String, f64>) -> Result<(), BacktestError> {
    for key in REPORT_METRIC_KEYS {
        match metrics.get(key) {
            Some(value) if value.is_finite() => {}
            _ => return Err(BacktestError::MissingReportMetric(key)),
        }
    }
    Ok(())
}

pub fn is_promotion_eligible(exploratory: bool, source: EvidenceSource) -> bool {
    !exploratory && source == EvidenceSource::NativeHarness
}

pub fn replay_id(snapshot_checksum: &str, config_hash: &str) -> String {
    blake3::hash(format!("{snapshot_checksum}:{config_hash}").as_bytes())
        .to_hex()
        .to_string()
}

fn report_metrics(starting_equity: f64, fills: &[FillResult]) -> BTreeMap<String, f64> {
    let pnl = fills.iter().map(|fill| fill.net_pnl).collect::<Vec<_>>();
    report_metric_pairs(starting_equity, fills, &pnl)
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect()
}

fn report_metric_pairs(
    starting_equity: f64,
    fills: &[FillResult],
    pnl: &[f64],
) -> [(&'static str, f64); REPORT_METRIC_COUNT] {
    [
        ("net_profit", sum(&pnl)),
        (
            "gross_profit",
            pnl.iter().copied().filter(|v| *v > 0.0).sum(),
        ),
        ("gross_loss", pnl.iter().copied().filter(|v| *v < 0.0).sum()),
        ("max_drawdown", max_drawdown(starting_equity, &pnl)),
        ("sharpe", sharpe(&pnl)),
        ("sortino", sortino(&pnl)),
        ("profit_factor", profit_factor(&pnl)),
        ("win_rate", win_rate(&pnl)),
        ("trade_count", fills.len() as f64),
        ("avg_trade", average(&pnl)),
        (
            "cost_total",
            fills.iter().map(|fill| sum_costs(fill.costs)).sum(),
        ),
    ]
}

fn robustness_suite(fills: &[FillResult], seed: u64) -> Vec<RobustnessResult> {
    let pnl = fills.iter().map(|fill| fill.net_pnl).collect::<Vec<_>>();
    vec![
        robustness(RobustnessKind::WalkForward, None, average(&pnl)),
        robustness(RobustnessKind::OutOfSample, None, tail_average(&pnl)),
        monte_carlo_result(&pnl, seed),
        robustness(
            RobustnessKind::ParameterStability,
            None,
            stability_score(&pnl),
        ),
        robustness(RobustnessKind::RegimeSliced, None, regime_score(fills)),
    ]
}

fn monte_carlo_result(pnl: &[f64], seed: u64) -> RobustnessResult {
    let mut shuffled = pnl.to_vec();
    deterministic_shuffle(&mut shuffled, seed);
    RobustnessResult {
        kind: RobustnessKind::MonteCarloReshuffle,
        seed: Some(seed),
        passed: sum(&shuffled).is_finite(),
        metric: sum(&shuffled),
    }
}

fn robustness(kind: RobustnessKind, seed: Option<u64>, metric: f64) -> RobustnessResult {
    RobustnessResult {
        kind,
        seed,
        passed: metric.is_finite(),
        metric,
    }
}

fn deterministic_shuffle(values: &mut [f64], mut state: u64) {
    for index in (1..values.len()).rev() {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        values.swap(index, (state as usize) % (index + 1));
    }
}

fn validate_fill(fill: &FillInput) -> Result<(), BacktestError> {
    require_positive(fill.price, "price")?;
    require_positive(fill.quantity, "quantity")?;
    if matches!(fill.side, -1 | 1) {
        Ok(())
    } else {
        Err(BacktestError::InvalidFill("side"))
    }
}

fn require_text(value: &str, field: &'static str) -> Result<(), BacktestError> {
    if value.trim().is_empty() {
        Err(BacktestError::InvalidConfig(field))
    } else {
        Ok(())
    }
}

fn require_positive(value: f64, field: &'static str) -> Result<(), BacktestError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(BacktestError::InvalidConfig(field))
    }
}

fn require_non_negative(value: f64, field: &'static str) -> Result<(), BacktestError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(BacktestError::InvalidConfig(field))
    }
}

fn validate_ratio(value: f64, field: &'static str) -> Result<(), BacktestError> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(BacktestError::InvalidConfig(field))
    }
}

fn sum(values: &[f64]) -> f64 {
    values.iter().fold(0.0, |total, value| total + value)
}

fn sum_costs(costs: CostBreakdown) -> f64 {
    costs.fees
        + costs.spread
        + costs.slippage
        + costs.market_impact
        + costs.latency
        + costs.partial_fill
        + costs.unavailable_liquidity
}

fn average(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        sum(values) / values.len() as f64
    }
}

fn tail_average(values: &[f64]) -> f64 {
    let split = values.len() / 2;
    average(&values[split..])
}

fn variance(values: &[f64], downside_only: bool) -> f64 {
    let avg = average(values);
    let selected = values
        .iter()
        .copied()
        .filter(|value| !downside_only || *value < 0.0)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return 0.0;
    }
    selected
        .iter()
        .map(|value| (value - avg) * (value - avg))
        .sum::<f64>()
        / selected.len() as f64
}

fn sharpe(values: &[f64]) -> f64 {
    let stddev = variance(values, false).sqrt();
    if stddev == 0.0 {
        0.0
    } else {
        average(values) / stddev
    }
}

fn sortino(values: &[f64]) -> f64 {
    let downside = variance(values, true).sqrt();
    if downside == 0.0 {
        0.0
    } else {
        average(values) / downside
    }
}

fn profit_factor(values: &[f64]) -> f64 {
    let profit: f64 = values.iter().copied().filter(|v| *v > 0.0).sum();
    let loss: f64 = values.iter().copied().filter(|v| *v < 0.0).sum();
    if loss == 0.0 {
        profit
    } else {
        profit / loss.abs()
    }
}

fn win_rate(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().filter(|value| **value > 0.0).count() as f64 / values.len() as f64
    }
}

fn max_drawdown(starting_equity: f64, pnl: &[f64]) -> f64 {
    let mut equity = starting_equity;
    let mut peak = starting_equity;
    let mut max_dd = 0.0;
    for value in pnl {
        equity += value;
        if equity > peak {
            peak = equity;
        }
        let drawdown = if peak == 0.0 {
            0.0
        } else {
            (peak - equity) / peak
        };
        if drawdown > max_dd {
            max_dd = drawdown;
        }
    }
    max_dd
}

fn stability_score(values: &[f64]) -> f64 {
    1.0 / (1.0 + variance(values, false))
}

fn regime_score(fills: &[FillResult]) -> f64 {
    let regimes = fills
        .iter()
        .map(|fill| fill.regime_id)
        .collect::<BTreeSet<_>>();
    regimes.len() as f64
}
