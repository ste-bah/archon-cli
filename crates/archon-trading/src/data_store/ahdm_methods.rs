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
    let native_strategy = ahdm_native_strategy();
    serde_json::json!({"schema_version": "archon-ahdm-backtest-config-v1", "config": config, "dataset": dataset_ref, "shared_rule_manifest": manifest, "native_strategy": native_strategy, "data_gate": gate, "config_hash": config.config_hash(), "manifest_hash": sha256_hex(manifest.to_string().as_bytes())})
}

fn ahdm_native_strategy() -> CustomOhlcvStrategy {
    CustomOhlcvStrategy {
        name: Some("AHDM-v1/shared-rule-manifest".into()),
        entry: vec![condition(
            indicator(OhlcvIndicator::ChangePct, None),
            ComparisonOp::Gte,
            constant(0.70),
        )],
        exit: vec![condition(
            indicator(OhlcvIndicator::ChangePct, None),
            ComparisonOp::Lte,
            constant(-0.55),
        )],
        min_hold_bars: 3,
    }
}

fn condition(left: OhlcvOperand, op: ComparisonOp, right: OhlcvOperand) -> OhlcvCondition {
    OhlcvCondition { left, op, right }
}

fn indicator(indicator: OhlcvIndicator, len: Option<usize>) -> OhlcvOperand {
    OhlcvOperand::Indicator { indicator, len }
}

fn constant(value: f64) -> OhlcvOperand {
    OhlcvOperand::Constant { value }
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
        self.write_ahdm_evidence_inventory_with_backtest_gate(generated_at, false)
    }

    pub fn write_ahdm_evidence_inventory_with_backtest_gate(
        &self,
        generated_at: &str,
        native_backtest_gate_passed: bool,
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
        let citations = ahdm_citations(generated_at, gaps.clone(), native_backtest_gate_passed);
        let evidence_dir = self.ahdm_strategy_root().join("evidence");
        let inventory_path = evidence_dir.join("kb-rule-inventory.md");
        let citations_path = evidence_dir.join("citations.json");
        let legacy_citations_path = self.ahdm_strategy_root().join("citations.json");
        validate_ahdm_citations(&citations)?;
        write_text(&inventory_path, &ahdm_inventory_markdown(&citations, &gaps))?;
        write_schema_json(&citations_path, &citations)?;
        write_schema_json(&legacy_citations_path, &citations)?;
        Ok(vec![inventory_path, citations_path, legacy_citations_path])
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
        let indicator = ahdm_pine_source("indicator", &manifest);
        let strategy = ahdm_pine_source("strategy", &manifest);
        write_text(&indicator_path, &indicator)?;
        write_text(&strategy_path, &strategy)?;
        let manifest_hash = sha256_hex(manifest.to_string().as_bytes());
        write_schema_json(
            &report_path,
            &serde_json::json!({
                "schema": "archon-ahdm-pine-compile-report-v1",
                "strategy_id": "AHDM-v1",
                "task_id": "TASK-TDL-120",
                "checked_at": generated_at,
                "tooling_available": true,
                "status": "mcp_invocation_capture_required_fail_closed",
                "promotion_eligible": false,
                "pine_results": "exploratory_only",
                "exploratory_only": true,
                "promotion_scope": "exploratory artifact generation only; generated reports require captured mcp__tradingview__* invocation results before compile readiness and paper/live promotion still requires independent readiness gates",
                "mcp_tools": [
                    "mcp__tradingview__pine_analyze",
                    "mcp__tradingview__pine_check",
                    "mcp__tradingview__pine_compile",
                    "mcp__tradingview__pine_smart_compile",
                    "mcp__tradingview__pine_get_errors",
                    "mcp__tradingview__pine_get_console"
                ],
                "tooling_unavailable_policy": {
                    "fail_closed": true,
                    "reason": "MCP tooling availability must be determined by actual mcp__tradingview__* invocation results recorded in tooling_results; generated placeholders are not promotion evidence."
                },
                "required_tooling": [
                    {"name": "pine_analyze", "available": true, "required_for_readiness": true, "captured_invocation_required": true},
                    {"name": "pine_check", "available": true, "required_for_readiness": true, "captured_invocation_required": true},
                    {"name": "pine_compile", "available": true, "required_for_readiness": true, "artifact_compile_proven": false, "captured_invocation_required": true},
                    {"name": "pine_get_console", "available": true, "required_for_readiness": false, "artifact_console_proven": false, "captured_invocation_required": true},
                    {"name": "pine_get_errors", "available": true, "required_for_readiness": false, "artifact_errors_proven": false, "captured_invocation_required": true},
                    {"name": "pine_smart_compile", "available": true, "required_for_readiness": true, "artifact_compile_proven": false, "captured_invocation_required": true}
                ],
                "tooling_results": [
                    {"tool": "mcp__tradingview__pine_analyze", "artifact": "AHDM-v1-indicator.pine", "invocation": "mcp__tradingview__pine_analyze({source: <AHDM-v1-indicator.pine>})", "capture_required": true, "promotion_evidence": false},
                    {"tool": "mcp__tradingview__pine_analyze", "artifact": "AHDM-v1-strategy.pine", "invocation": "mcp__tradingview__pine_analyze({source: <AHDM-v1-strategy.pine>})", "capture_required": true, "promotion_evidence": false},
                    {"tool": "mcp__tradingview__pine_check", "artifact": "AHDM-v1-indicator.pine", "invocation": "mcp__tradingview__pine_check({source: <AHDM-v1-indicator.pine>})", "capture_required": true, "promotion_evidence": false},
                    {"tool": "mcp__tradingview__pine_check", "artifact": "AHDM-v1-strategy.pine", "invocation": "mcp__tradingview__pine_check({source: <AHDM-v1-strategy.pine>})", "capture_required": true, "promotion_evidence": false},
                    {"tool": "mcp__tradingview__pine_compile", "artifact": "current TradingView editor must be proven to be AHDM artifact", "invocation": "mcp__tradingview__pine_compile()", "capture_required": true, "promotion_evidence": false},
                    {"tool": "mcp__tradingview__pine_smart_compile", "artifact": "current TradingView editor must be proven to be AHDM artifact", "invocation": "mcp__tradingview__pine_smart_compile()", "capture_required": true, "promotion_evidence": false},
                    {"tool": "mcp__tradingview__pine_get_errors", "artifact": "current TradingView editor must be proven to be AHDM artifact", "invocation": "mcp__tradingview__pine_get_errors()", "capture_required": true, "promotion_evidence": false},
                    {"tool": "mcp__tradingview__pine_get_console", "artifact": "current TradingView editor must be proven to be AHDM artifact", "invocation": "mcp__tradingview__pine_get_console()", "capture_required": true, "promotion_evidence": false}
                ],
                "residual_gaps": [
                    {
                        "id": "GAP-AHDM-PINE-CHART-COMPILE-001",
                        "task_id": "TASK-TDL-120",
                        "area": "pine_chart_compile_readiness",
                        "description": "Generated Pine artifacts require captured mcp__tradingview__* invocation returned_result or captured_error values before compile readiness; placeholder invocation entries fail closed and are not promotion evidence.",
                        "severity": "blocker",
                        "captured_mcp_invocations": [
                            "mcp__tradingview__pine_analyze({source: <AHDM-v1-indicator.pine>})",
                            "mcp__tradingview__pine_analyze({source: <AHDM-v1-strategy.pine>})",
                            "mcp__tradingview__pine_check({source: <AHDM-v1-indicator.pine>})",
                            "mcp__tradingview__pine_check({source: <AHDM-v1-strategy.pine>})",
                            "mcp__tradingview__pine_compile()",
                            "mcp__tradingview__pine_smart_compile()",
                            "mcp__tradingview__pine_get_errors()",
                            "mcp__tradingview__pine_get_console()"
                        ],
                        "fail_closed": true,
                        "fail_closed_behavior": "Pine artifacts remain exploratory and do not satisfy Pine readiness or paper/live promotion evidence without complete Pine tooling evidence and independent readiness gates.",
                        "blocks": ["pine_readiness", "paper_trading_promotion", "live_trading_promotion"],
                        "owner": "Archon",
                        "created_at": generated_at
                    }
                ],
                "shared_manifest_traceability": {
                    "native_and_pine_parity_key": manifest["native_and_pine_parity_key"].clone(),
                    "native_manifest_source": "crates/archon-trading/src/data_store/ahdm.rs::ahdm_rule_manifest",
                    "pine_generation_source": "crates/archon-trading/src/data_store/ahdm_methods.rs::write_ahdm_pine_artifacts",
                    "shared_manifest_hash": manifest_hash,
                    "indicator": indicator_path.to_string_lossy(),
                    "strategy": strategy_path.to_string_lossy(),
                    "indicator_sha256": sha256_hex(indicator.as_bytes()),
                    "strategy_sha256": sha256_hex(strategy.as_bytes()),
                    "indicator_line_count": indicator.lines().count(),
                    "strategy_line_count": strategy.lines().count()
                },
                "promotion": {
                    "pine_readiness": "blocked_by_chart_compile_gap",
                    "paper_trading_readiness": "requires_independent_native_gates",
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
        let report =
            run_ahdm_shared_manifest_backtest(&config, &request, &dataset.bars).map_err(|err| {
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
        let manifest_hash = sha256_hex(manifest.to_string().as_bytes());
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
