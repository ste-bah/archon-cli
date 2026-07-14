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
            diagnostic_allow_degraded_data,
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
            let lake = TradingDataLake::new(root);
            if strategy_rules.is_some() && !*diagnostic_allow_degraded_data {
                return Err(anyhow!(
                    "promotion OHLCV backtests require dataset id/version strategy inputs; loose strategy-rules paths are diagnostic-only"
                ));
            }
            let gate = lake
                .backtest_data_gate(dataset_id, version, *diagnostic_allow_degraded_data)
                .map_err(|err| anyhow!("OHLCV backtest data gate failed: {err:?}"))?;
            let dataset = lake
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
            let mut value = serde_json::to_value(&report)?;
            if let serde_json::Value::Object(map) = &mut value {
                map.insert("data_gate".into(), serde_json::to_value(gate)?);
                map.insert(
                    "dataset_provenance".into(),
                    dataset_provenance(&dataset.record),
                );
            }
            write_or_render(&value, out.as_deref())
        }
        TradingCliBacktestAction::RunAhdmNative {
            config,
            target,
            run_id,
            dataset_id,
            version,
            quantity,
            generated_at,
            out,
        } => {
            let config: BacktestConfig = read_json(config, "BacktestConfig")?;
            let root = project_root(target.as_ref())?;
            let lake = TradingDataLake::new(&root);
            let generated_at = generated_at
                .clone()
                .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
            let run_dir = lake
                .run_ahdm_native_backtest(
                    run_id,
                    dataset_id,
                    version,
                    config,
                    *quantity,
                    &generated_at,
                )
                .map_err(|err| anyhow!("AHDM native backtest failed: {err:?}"))?;
            let dataset = lake.load_ohlcv(dataset_id, version).map_err(|err| {
                anyhow!("failed to load AHDM backtest dataset provenance: {err:?}")
            })?;
            let report = serde_json::json!({
                "status": "created",
                "strategy_id": "AHDM-v1",
                "run_id": run_id,
                "dataset_id": dataset_id,
                "version": version,
                "generated_at": generated_at,
                "dataset_provenance": dataset_provenance(&dataset.record),
                "run_dir": run_dir,
                "artifacts": {
                    "config": run_dir.join("config.json"),
                    "report": run_dir.join("report.json"),
                    "trades": run_dir.join("trades.jsonl"),
                    "equity_curve": run_dir.join("equity_curve.jsonl"),
                    "adversarial_review": run_dir.join("adversarial-review.md")
                }
            });
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

fn dataset_provenance(
    record: &archon_trading::data_store::StoredDatasetRecord,
) -> serde_json::Value {
    serde_json::json!({
        "dataset_id": record.dataset_id,
        "version": record.version,
        "provider": record.provider,
        "timeframe": record.timeframe,
        "status": record.status,
        "checksum": record.checksum,
        "metadata_checksum": record.metadata_checksum,
        "validation_path": record.validation_path,
        "manifest_path": record.manifest_path,
        "metadata_path": record.metadata_path,
        "normalized_path": record.normalized_path,
        "raw_path": record.raw_path,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_args::TradingCliBacktestAction;
    use std::path::PathBuf;

    #[test]
    fn dataset_provenance_records_gate_identifiers_and_manifest_path() {
        let record = stored_record();
        let provenance = dataset_provenance(&record);

        assert_eq!(provenance["dataset_id"], "btc-1d");
        assert_eq!(provenance["version"], "v1");
        assert_eq!(provenance["provider"], "tradingview");
        assert_eq!(provenance["timeframe"], "1D");
        assert_eq!(provenance["validation_path"], "validation.json");
        assert_eq!(provenance["manifest_path"], "manifest.json");
    }

    #[test]
    fn run_ohlcv_refuses_loose_strategy_rules_for_promotion_backtest() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("backtest.json");
        std::fs::write(&config, serde_json::to_string(&config_fixture()).unwrap()).unwrap();

        let result = render_backtest(&TradingCliBacktestAction::RunOhlcv {
            config,
            target: Some(temp.path().to_path_buf()),
            dataset_id: "btc-1d".into(),
            version: "v1".into(),
            diagnostic_allow_degraded_data: false,
            quantity: 1.0,
            rule: TradingCliOhlcvRule::CloseMomentum,
            strategy_rules: Some(PathBuf::from("loose-rules.json")),
            fast_len: 10,
            slow_len: 30,
            exploratory: false,
            source: TradingCliBacktestSource::NativeHarness,
            out: None,
        });

        assert!(matches!(
            result,
            Err(error) if error.to_string().contains("loose strategy-rules paths")
        ));
    }

    fn stored_record() -> archon_trading::data_store::StoredDatasetRecord {
        archon_trading::data_store::StoredDatasetRecord {
            dataset_id: "btc-1d".into(),
            version: "v1".into(),
            schema_version: "archon-trading-data-registry-v2".into(),
            dataset_path: "datasets/btc-1d/v1".into(),
            metadata_checksum: "metadata-checksum".into(),
            raw_checksum: "raw-checksum".into(),
            raw_response_path: "raw/response.json".into(),
            raw_request_path: "raw/request.json".into(),
            redacted_headers_path: "raw/headers.redacted.json".into(),
            provider_notes_path: "raw/provider-notes.md".into(),
            provider: "tradingview".into(),
            data_type: "Ohlcv".into(),
            symbol: "BTCUSD".into(),
            timeframe: "1D".into(),
            native_interval: true,
            production_eligible: true,
            status: DatasetStatus::Healthy,
            checksum: "normalized-checksum".into(),
            bars: 2,
            coverage_start: "2026-01-01T00:00:00Z".into(),
            coverage_end: "2026-01-02T00:00:00Z".into(),
            metadata_path: "metadata.json".into(),
            normalized_path: "ohlcv.jsonl".into(),
            raw_path: "raw/response.json".into(),
            validation_path: "validation.json".into(),
            manifest_path: "manifest.json".into(),
            created_at: "2026-01-02T00:00:00Z".into(),
        }
    }

    fn config_fixture() -> BacktestConfig {
        BacktestConfig {
            strategy_id: "strategy-1".into(),
            snapshot_checksum: "checksum".into(),
            starting_equity: 10_000.0,
            fee_per_share: 0.0,
            spread_bps: 0.0,
            slippage_bps: 0.0,
            market_impact_bps: 0.0,
            latency_ms: 0,
            partial_fill_ratio: 1.0,
            unavailable_liquidity_ratio: 0.0,
            monte_carlo_seed: 1,
            parameter_set_id: "params-1".into(),
        }
    }
}
