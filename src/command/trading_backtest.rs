use anyhow::{Result, anyhow};
use archon_trading::backtest::{BacktestConfig, BacktestHarness, EvidenceSource, FillInput};
use archon_trading::candle_backtest::{run_custom_ohlcv_backtest, run_ohlcv_backtest};
use archon_trading::custom_strategy::CustomOhlcvStrategy;
use archon_trading::data_lake::DatasetStatus;
use archon_trading::data_store::TradingDataLake;
use archon_trading::ohlcv::{OhlcvBacktestRequest, OhlcvBacktestRule, OhlcvDatasetRef};

use crate::cli_args::{
    TradingCliBacktestAction, TradingCliBacktestSource, TradingCliDatasetStatus,
    TradingCliOhlcvRule,
};
use crate::command::trading_io::{read_json, write_or_render};
use crate::command::trading_tools::project_root;

pub(crate) fn render_backtest(action: &TradingCliBacktestAction) -> Result<String> {
    match action {
        TradingCliBacktestAction::Run {
            config,
            fills,
            dataset_status,
            exploratory,
            source,
            out,
        } => {
            let config: BacktestConfig = read_json(config, "BacktestConfig")?;
            let fills: Vec<FillInput> = read_json(fills, "FillInput[]")?;
            let report = BacktestHarness::new(config)
                .map_err(|err| anyhow!("invalid backtest config: {err:?}"))?
                .run(
                    &fills,
                    (*dataset_status).into(),
                    *exploratory,
                    (*source).into(),
                )
                .map_err(|err| anyhow!("backtest failed: {err:?}"))?;
            write_or_render(&report, out.as_deref())
        }
        TradingCliBacktestAction::RunOhlcv {
            config,
            target,
            dataset_id,
            version,
            quantity,
            rule,
            strategy_rules,
            fast_len,
            slow_len,
            exploratory,
            source,
            out,
        } => {
            let config: BacktestConfig = read_json(config, "BacktestConfig")?;
            let root = project_root(target.as_ref())?;
            let dataset = TradingDataLake::new(root)
                .load_ohlcv(dataset_id, version)
                .map_err(|err| anyhow!("failed to load OHLCV dataset: {err:?}"))?;
            let request = request(
                &dataset.record,
                *quantity,
                *rule,
                *fast_len,
                *slow_len,
                *exploratory,
                *source,
            );
            let report = if let Some(path) = strategy_rules {
                let strategy: CustomOhlcvStrategy = read_json(path, "CustomOhlcvStrategy")?;
                run_custom_ohlcv_backtest(&config, &request, &dataset.bars, &strategy)
            } else {
                run_ohlcv_backtest(&config, &request, &dataset.bars)
            }
            .map_err(|err| anyhow!("OHLCV backtest failed: {err:?}"))?;
            write_or_render(&report, out.as_deref())
        }
    }
}

fn request(
    record: &archon_trading::data_store::StoredDatasetRecord,
    quantity: f64,
    rule: TradingCliOhlcvRule,
    fast_len: usize,
    slow_len: usize,
    exploratory: bool,
    source: TradingCliBacktestSource,
) -> OhlcvBacktestRequest {
    OhlcvBacktestRequest {
        dataset: OhlcvDatasetRef {
            dataset_id: record.dataset_id.clone(),
            version: record.version.clone(),
            checksum: record.checksum.clone(),
            status: record.status,
        },
        rule: rule.into(),
        quantity,
        exploratory,
        source: source.into(),
        fast_len,
        slow_len,
    }
}

impl From<TradingCliDatasetStatus> for DatasetStatus {
    fn from(value: TradingCliDatasetStatus) -> Self {
        match value {
            TradingCliDatasetStatus::Healthy => Self::Healthy,
            TradingCliDatasetStatus::Degraded => Self::Degraded,
        }
    }
}

impl From<TradingCliBacktestSource> for EvidenceSource {
    fn from(value: TradingCliBacktestSource) -> Self {
        match value {
            TradingCliBacktestSource::NativeHarness => Self::NativeHarness,
            TradingCliBacktestSource::StrategyTester => Self::StrategyTester,
        }
    }
}

impl From<TradingCliOhlcvRule> for OhlcvBacktestRule {
    fn from(value: TradingCliOhlcvRule) -> Self {
        match value {
            TradingCliOhlcvRule::CloseMomentum => Self::CloseMomentum,
            TradingCliOhlcvRule::SmaCross => Self::SmaCross,
        }
    }
}
