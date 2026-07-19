use super::*;

fn required_cell_field<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, DataStoreError> {
    value
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| DataStoreError::InvalidMetadata(format!("coverage cell missing {field}")))
}

fn coverage_run_id(run_prefix: &str, cell: &CoverageCell) -> String {
    format!(
        "{}-{}-{}",
        safe_path(run_prefix),
        safe_path(&cell.canonical_instrument),
        safe_path(&cell.timeframe)
    )
}
fn ahdm_backtest_request(record: &StoredDatasetRecord, quantity: f64) -> OhlcvBacktestRequest {
    OhlcvBacktestRequest {
        dataset: OhlcvDatasetRef {
            dataset_id: record.dataset_id.clone(),
            version: record.version.clone(),
            checksum: record.checksum.clone(),
            status: record.status,
        },
        rule: OhlcvBacktestRule::CloseMomentum,
        quantity,
        exploratory: false,
        source: EvidenceSource::NativeHarness,
        fast_len: 10,
        slow_len: 30,
    }
}

fn ahdm_backtest_config(
    config: &BacktestConfig,
    manifest: &serde_json::Value,
    gate: &BacktestDataGateReport,
    dataset_ref: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({"schema_version": "archon-ahdm-backtest-config-v1", "config": config, "dataset": dataset_ref, "shared_rule_manifest": manifest, "data_gate": gate, "config_hash": config.config_hash(), "manifest_hash": bytes_checksum(manifest.to_string().as_bytes())})
}

fn ahdm_backtest_report(
    report: &OhlcvBacktestReport,
    manifest_hash: &str,
    gate: &BacktestDataGateReport,
    dataset_ref: &serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({"schema_version": "archon-ahdm-backtest-report-v1", "report": report, "dataset": dataset_ref, "data_gate": gate, "shared_rule_manifest_hash": manifest_hash, "diagnostic": false, "promotion_eligible": report.promotion_eligible && gate.promotion_eligible})
}

fn ahdm_backtest_dataset_ref(dataset: &StoredOhlcvDataset) -> serde_json::Value {
    serde_json::json!({
        "dataset_id": dataset.record.dataset_id,
        "version": dataset.record.version,
        "provider": dataset.record.provider,
        "timeframe": dataset.metadata.timeframe,
        "native_interval": dataset.metadata.native_interval,
        "production_eligible": dataset.metadata.production_eligible,
        "validation_path": dataset.record.validation_path,
        "normalized_path": dataset.record.normalized_path,
        "raw_path": dataset.record.raw_path,
    })
}

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
        write_schema_json(&citations_path, &citations)?;
        Ok(vec![inventory_path, citations_path])
    }

    pub fn write_ahdm_strategy_spec(&self, generated_at: &str) -> Result<PathBuf, DataStoreError> {
        let spec = ahdm_strategy_spec(&self.load_registry()?, generated_at);
        let path = self.ahdm_strategy_root().join("strategy-spec.json");
        write_schema_json(&path, &spec)?;
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
        write_schema_json(
            &report_path,
            &serde_json::json!({
                "schema": "archon-ahdm-pine-compile-report-v1",
                "strategy_id": "AHDM-v1",
                "task_id": "TASK-TDL-120",
                "checked_at": generated_at,
                "tooling_available": false,
                "status": "tooling_unavailable",
                "promotion_eligible": false,
                "pine_results": "exploratory_only",
                "exploratory_only": true,
                "promotion_scope": "exploratory artifact generation only; Pine tooling evidence is required before compile readiness and paper/live promotion still requires independent readiness gates",
                "mcp_tools": [],
                "tooling_unavailable_policy": {
                    "fail_closed": true,
                    "reason": "write_ahdm_pine_artifacts does not call provider-sensitive Pine tooling; external compile evidence must be recorded separately"
                },
                "required_tooling": [
                    {"name": "pine_analyze", "available": false, "required_for_readiness": true},
                    {"name": "pine_check", "available": false, "required_for_readiness": true},
                    {"name": "pine_compile", "available": false, "required_for_readiness": true},
                    {"name": "pine_get_console", "available": false, "required_for_readiness": false},
                    {"name": "pine_get_errors", "available": false, "required_for_readiness": false},
                    {"name": "pine_smart_compile", "available": false, "required_for_readiness": true}
                ],
                "residual_gaps": [
                    {
                        "id": "GAP-AHDM-PINE-TOOLING-001",
                        "task_id": "TASK-TDL-120",
                        "area": "pine_compile_readiness",
                        "description": "Pine static analysis/check/compile evidence has not been supplied by TradingView tooling for these generated artifacts.",
                        "severity": "blocker",
                        "fail_closed": true,
                        "fail_closed_behavior": "Pine readiness and paper/live promotion evidence remain unsatisfied until external Pine tooling records successful checks for the exact artifact hashes.",
                        "blocks": ["pine_readiness", "paper_trading_promotion", "live_trading_promotion"],
                        "owner": "Archon",
                        "created_at": generated_at
                    }
                ],
                "shared_manifest_traceability": {
                    "native_and_pine_parity_key": manifest["native_and_pine_parity_key"].clone(),
                    "native_manifest_source": "crates/archon-trading/src/data_store/ahdm.rs::ahdm_rule_manifest",
                    "pine_generation_source": "crates/archon-trading/src/data_store/ahdm_methods.rs::write_ahdm_pine_artifacts",
                    "indicator": indicator_path.to_string_lossy(),
                    "strategy": strategy_path.to_string_lossy()
                },
                "promotion": {
                    "pine_readiness": "requires_external_compile_check",
                    "paper_trading_readiness": "requires_independent_gates",
                    "live_trading_enabled": false,
                    "reason": "Generated Pine artifacts are exploratory; unavailable Pine tooling evidence fails closed and does not satisfy paper/live promotion gates."
                }
            }),
        )?;
        Ok(vec![indicator_path, strategy_path, report_path])
    }

    pub fn run_ahdm_native_backtest_coverage(
        &self,
        run_prefix: &str,
        config: BacktestConfig,
        quantity: f64,
        generated_at: &str,
    ) -> Result<Vec<PathBuf>, DataStoreError> {
        let matrix = self.coverage_matrix("trading-core-v1", generated_at.into())?;
        validate_coverage_matrix_complete(&matrix)?;
        let mut seen = std::collections::BTreeSet::new();
        let mut run_dirs = Vec::new();
        for cell in matrix.cells.iter().filter(|cell| cell.available) {
            let dataset_id = required_cell_field(cell.dataset_id.as_deref(), "dataset_id")?;
            let version = required_cell_field(cell.version.as_deref(), "version")?;
            if !seen.insert((dataset_id.to_string(), version.to_string())) {
                continue;
            }
            run_dirs.push(self.run_ahdm_native_backtest(
                &coverage_run_id(run_prefix, cell),
                dataset_id,
                version,
                config.clone(),
                quantity,
                generated_at,
            )?);
        }
        if run_dirs.is_empty() {
            return Err(DataStoreError::InvalidMetadata(
                "AHDM coverage backtest found no registered datasets".into(),
            ));
        }
        Ok(run_dirs)
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
        write_schema_json(
            &dir.join("config.json"),
            &ahdm_backtest_config(&config, &manifest, &gate, &dataset_ref),
        )?;
        let manifest_hash = bytes_checksum(manifest.to_string().as_bytes());
        write_schema_json(
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
