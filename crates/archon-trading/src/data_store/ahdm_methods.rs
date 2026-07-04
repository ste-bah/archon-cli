use super::*;

impl TradingDataLake {
    pub fn write_ahdm_evidence_inventory(
        &self,
        generated_at: &str,
    ) -> Result<Vec<PathBuf>, DataStoreError> {
        let coverage = self.coverage_matrix("trading-core-v1", generated_at.into())?;
        let gaps = coverage
            .gaps
            .iter()
            .map(|gap| {
                format!(
                    "{} {}: {}",
                    gap.canonical_instrument, gap.timeframe, gap.reason
                )
            })
            .collect::<Vec<_>>();
        let citations = ahdm_citations(generated_at, gaps.clone());
        let evidence_dir = self.ahdm_strategy_root().join("evidence");
        let inventory_path = evidence_dir.join("kb-rule-inventory.md");
        let citations_path = evidence_dir.join("citations.json");
        validate_ahdm_citations(&citations)?;
        write_text(&inventory_path, &ahdm_inventory_markdown(&citations, &gaps))?;
        write_json(&citations_path, &citations)?;
        Ok(vec![inventory_path, citations_path])
    }

    pub fn write_ahdm_strategy_spec(&self, generated_at: &str) -> Result<PathBuf, DataStoreError> {
        let spec = ahdm_strategy_spec(&self.load_registry()?, generated_at);
        let path = self.ahdm_strategy_root().join("strategy-spec.json");
        write_json(&path, &spec)?;
        Ok(path)
    }

    pub fn write_ahdm_pine_artifacts(
        &self,
        generated_at: &str,
    ) -> Result<Vec<PathBuf>, DataStoreError> {
        let spec_path = self.write_ahdm_strategy_spec(generated_at)?;
        let spec: serde_json::Value = read_json(&spec_path)?;
        let manifest = spec
            .get("shared_rule_manifest")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let dir = self.ahdm_strategy_root().join("pine");
        let indicator_path = dir.join("AHDM-v1-indicator.pine");
        let strategy_path = dir.join("AHDM-v1-strategy.pine");
        let report_path = dir.join("compile-report.json");
        write_text(&indicator_path, &ahdm_pine_source("indicator", &manifest))?;
        write_text(&strategy_path, &ahdm_pine_source("strategy", &manifest))?;
        write_json(
            &report_path,
            &serde_json::json!({
                "schema_version": "archon-ahdm-pine-compile-report-v1",
                "strategy_id": "AHDM-v1",
                "checked_at": generated_at,
                "tooling_available": false,
                "status": "not_checked_fail_closed",
                "promotion_eligible": false,
                "pine_results": "exploratory_only",
                "residual_gap": residual_gap("GAP-AHDM-PINE-001", "pine", "TradingView/Pine compile tooling is unavailable in this offline run", "Pine readiness and promotion evidence cannot be satisfied", generated_at),
            }),
        )?;
        Ok(vec![indicator_path, strategy_path, report_path])
    }

    pub fn run_ahdm_native_backtest(
        &self,
        run_id: &str,
        dataset_id: &str,
        version: &str,
        config: BacktestConfig,
        quantity: f64,
        generated_at: &str,
    ) -> Result<PathBuf, DataStoreError> {
        let gate = self.backtest_data_gate(dataset_id, version, false)?;
        let dataset = self.load_ohlcv(dataset_id, version)?;
        let request = ahdm_backtest_request(&dataset.record, quantity);
        let report = run_ohlcv_backtest(&config, &request, &dataset.bars).map_err(|err| {
            DataStoreError::InvalidMetadata(format!("AHDM backtest failed: {err:?}"))
        })?;
        let dir = self
            .ahdm_strategy_root()
            .join("backtests")
            .join(safe_path(run_id));
        let manifest = ahdm_rule_manifest(generated_at);
        let dataset_ref = ahdm_backtest_dataset_ref(&dataset);
        write_json(
            &dir.join("config.json"),
            &ahdm_backtest_config(&config, &manifest, &gate, &dataset_ref),
        )?;
        let manifest_hash = bytes_checksum(manifest.to_string().as_bytes());
        write_json(
            &dir.join("report.json"),
            &ahdm_backtest_report(&report, &manifest_hash, &gate, &dataset_ref),
        )?;
        write_jsonl_trades(&dir.join("trades.jsonl"), &report.trades)?;
        write_equity_curve(
            &dir.join("equity_curve.jsonl"),
            config.starting_equity,
            &report,
        )?;
        self.write_ahdm_adversarial_review(run_id, generated_at)?;
        Ok(dir)
    }

    pub fn write_ahdm_adversarial_review(
        &self,
        run_id: &str,
        generated_at: &str,
    ) -> Result<PathBuf, DataStoreError> {
        let dir = self
            .ahdm_strategy_root()
            .join("backtests")
            .join(safe_path(run_id));
        let report_path = dir.join("report.json");
        let report = if report_path.exists() {
            read_json::<serde_json::Value>(&report_path)?
        } else {
            serde_json::json!({"promotion_eligible": false})
        };
        let path = dir.join("adversarial-review.md");
        write_text(
            &path,
            &ahdm_adversarial_review_markdown(run_id, &report, generated_at),
        )?;
        Ok(path)
    }

    pub fn write_ahdm_paper_trading_readiness(
        &self,
        generated_at: &str,
    ) -> Result<PathBuf, DataStoreError> {
        let path = self
            .ahdm_strategy_root()
            .join("readiness/paper-trading-readiness.md");
        write_text(&path, &ahdm_readiness_report(self, generated_at)?)?;
        Ok(path)
    }
}
