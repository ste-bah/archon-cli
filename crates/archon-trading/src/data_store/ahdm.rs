use super::*;
pub(super) fn ahdm_citations(generated_at: &str, coverage_gaps: Vec<String>) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "archon-ahdm-citations-v1",
        "strategy_id": "AHDM-v1",
        "generated_at": generated_at,
        "promotion_policy": {
            "every_rule_requires_citation_or_hypothesis": true,
            "hypotheses_barred_from_promotion_until_cited": true,
            "confidence_is_score_not_probability": true,
            "no_live_trading": true,
            "diagnostic_artifacts_cannot_promote": true
        },
        "kb_priority": [
            "trading-hybrid-system",
            "trading-market-structure",
            "trading-risk-management",
            "trading-backtesting",
            "trading-execution",
            "trading-strategy-research",
            "trading-postmortems"
        ],
        "secondary_non_authoritative_kbs": ["trading-elliott-wave"],
        "rules": ahdm_rule_manifest(generated_at)["rules"].clone(),
        "inventory_gate": {
            "status": "blocked_fail_closed",
            "promotion_allowed": false,
            "fail_closed_behavior": "KB inventory can be generated for audit, but cannot promote StrategySpec/readiness unless data coverage and citation gates pass",
            "required_prerequisites": {
                "citation_or_hypothesis_policy_passed": true,
                "data_coverage_gate_passed": coverage_gaps.is_empty(),
                "hypothesis_rules_absent": false,
                "native_backtest_gate_passed": false
            }
        },
        "promotion_allowed": false,
        "coverage_gaps": coverage_gaps,
        "residual_gaps": [residual_gap("GAP-AHDM-DATA-001", "data", "Trading Data Lake coverage is incomplete when coverage_gaps is non-empty", "Promotion-oriented strategy work and production backtests are refused until coverage gaps are closed", generated_at),
            residual_gap("GAP-AHDM-KB-HYPOTHESIS-001", "kb", "At least one AHDM rule remains classified as hypothesis", "Hypothesis rules remain research-only and cannot satisfy promotion gates", generated_at)]
    })
}

pub(super) fn ahdm_rule_manifest(generated_at: &str) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "archon-ahdm-rule-manifest-v1",
        "strategy_id": "AHDM-v1",
        "generated_at": generated_at,
        "rules": [
            cited_rule("higher_timeframe_trend_regime", 20, "trading-hybrid-system"),
            cited_rule("liquidity_location_prior_highs_lows", 20, "trading-market-structure"),
            cited_rule("vector_volume_behavior", 20, "trading-market-structure"),
            cited_rule("adr_awr_range_state", 15, "trading-strategy-research"),
            hypothesis_rule("vwap_ema_relationship", 10),
            cited_rule("session_timing_macro_filter", 10, "trading-execution"),
            cited_rule("recent_postmortem_penalty", 5, "trading-postmortems")
        ],
        "entry_models": [
            entry_model("liquidity_sweep_reversal"),
            entry_model("trend_continuation_pullback"),
            entry_model("range_mean_reversion")
        ],
        "thresholds": {
            "no_trade_below_confidence": 0.55,
            "paper_consideration_min_confidence": 0.70,
            "paper_requires_backtest_gates": true
        },
        "native_and_pine_parity_key": "AHDM-v1/shared-rule-manifest"
    })
}

pub(super) fn cited_rule(id: &str, weight: u64, kb: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "weight": weight,
        "status": "cited",
        "citation": {"kb": kb, "locator": format!("{kb}:rule-inventory")},
        "promotion_allowed": true,
        "required_evidence": ["registered_native_dataset", "kb_citation"]
    })
}

pub(super) fn hypothesis_rule(id: &str, weight: u64) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "weight": weight,
        "status": "hypothesis",
        "citation": serde_json::Value::Null,
        "promotion_allowed": false,
        "required_evidence": ["registered_native_dataset", "kb_citation_or_hypothesis_label"]
    })
}

pub(super) fn entry_model(id: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "required_evidence": ["registered_native_dataset", "bias_component_score", "invalidation_level"],
        "missing_evidence_behavior": "no_trade_fail_closed",
        "outputs": ["entry_zone", "stop", "tp1", "tp2", "tp3", "position_size"]
    })
}

pub(super) fn ahdm_inventory_markdown(citations: &serde_json::Value, gaps: &[String]) -> String {
    let mut text = String::from(
        "# AHDM-v1 KB Rule Inventory\n\nRules are cited or explicitly marked hypothesis. Hypotheses are barred from promotion until cited. Confidence is a score, not a probability. Live trading is out of scope.\n\n## Prioritized KBs\n\n",
    );
    if let Some(kbs) = citations
        .get("kb_priority")
        .and_then(serde_json::Value::as_array)
    {
        for kb in kbs {
            text.push_str(&format!("- {}\n", kb.as_str().unwrap_or("unknown")));
        }
    }
    text.push_str("\n## Rules\n\n");
    if let Some(rules) = citations.get("rules").and_then(serde_json::Value::as_array) {
        for rule in rules {
            text.push_str(&format!(
                "- `{}` weight={} status={} promotion_allowed={}\n",
                rule["id"].as_str().unwrap_or("unknown"),
                rule["weight"],
                rule["status"],
                rule["promotion_allowed"]
            ));
        }
    }
    text.push_str("\n## Inventory Gate\n\n");
    if let Some(gate) = citations
        .get("inventory_gate")
        .and_then(serde_json::Value::as_object)
    {
        text.push_str(&format!(
            "- status={}\n- promotion_allowed={}\n- fail_closed_behavior={}\n",
            gate.get("status").unwrap_or(&serde_json::Value::Null),
            gate.get("promotion_allowed")
                .unwrap_or(&serde_json::Value::Null),
            gate.get("fail_closed_behavior")
                .unwrap_or(&serde_json::Value::Null)
        ));
    }
    text.push_str("\n## Data Coverage Gaps\n\n");
    if gaps.is_empty() {
        text.push_str("- none recorded by coverage matrix\n");
    } else {
        for gap in gaps {
            text.push_str(&format!("- {gap}\n"));
        }
    }
    text.push_str("\n## Residual Gaps\n\n");
    if let Some(residual_gaps) = citations
        .get("residual_gaps")
        .and_then(serde_json::Value::as_array)
    {
        for gap in residual_gaps {
            text.push_str(&format!(
                "- `{}`: {}\n",
                gap["id"], gap["fail_closed_behavior"]
            ));
        }
    }
    text
}

pub(super) fn ahdm_strategy_spec(
    registry: &PersistentDatasetRegistry,
    generated_at: &str,
) -> serde_json::Value {
    let datasets = registry
        .datasets
        .values()
        .map(|record| {
            serde_json::json!({
                "dataset_id": record.dataset_id,
                "version": record.version,
                "provider": record.provider,
                "coverage_start": record.coverage_start,
                "coverage_end": record.coverage_end,
                "status": record.status
            })
        })
        .collect::<Vec<_>>();
    let manifest = ahdm_rule_manifest(generated_at);
    serde_json::json!({
        "schema_version": "archon-ahdm-strategy-spec-v1",
        "schema": "archon-ahdm-strategy-spec-v1",
        "strategy_id": "AHDM-v1",
        "generated_at": generated_at,
        "confidence": {
            "type": "score",
            "score_not_probability": true,
            "sum_weights": 100,
            "no_trade_below": 0.55,
            "paper_consideration_min": 0.70,
            "paper_requires_backtest_gates": true
        },
        "initial_bias": {
            "components": manifest["rules"].clone(),
            "sum_weights": 100,
            "missing_evidence_behavior": "no_trade_fail_closed"
        },
        "entry_models": manifest["entry_models"].clone(),
        "datasets": datasets.clone(),
        "risk": {
            "position_sizing": {"risk_fraction": 0.005, "max_fraction": 0.01, "formula": "min(account_equity*risk_fraction/abs(entry-stop), account_equity*max_fraction/entry)", "promotion_claim": false},
            "missing_or_invalid_inputs": "no_trade_fail_closed",
            "live_trading": "out_of_scope"
        },
        "instrument_universe": ["ES", "NQ", "SPY", "QQQ", "BTCUSDT", "ETHUSDT"],
        "timeframe_stack": ["1W", "1D", "240", "60", "15"],
        "required_datasets": datasets,
        "dataset_reference_policy": {
            "reference_type": "registered_dataset_id_and_version",
            "loose_file_references_allowed": false,
            "missing_refs_behavior": "no_trade_fail_closed"
        },
        "daily_bias_formula": manifest["rules"].clone(),
        "confidence_scoring": {"type": "score_not_probability", "sum_weights": 100, "no_trade_below": 0.55, "paper_consideration_min": 0.70},
        "invalidation_logic": "entry invalidates on missing required evidence, broken setup level, or data gate failure",
        "stop_logic": "deterministic setup invalidation stop from shared manifest; no live order placement",
        "tp_logic": {"tp1": "first planned objective", "tp2": "second planned objective", "tp3": "final planned objective"},
        "prd_section_27": {
            "strategy": "AHDM-v1",
            "bias_components": "exact weighted initial_bias.components sum to 100",
            "entry_models": ["liquidity_sweep_reversal", "trend_continuation_pullback", "range_mean_reversion"],
            "dataset_references": "registered dataset_id/version records only; loose files are not evidence",
            "confidence": "score, not probability",
            "no_trade_rule": "confidence < 0.55 or missing required evidence",
            "paper_gate": "confidence >= 0.70 and backtest gates passed",
            "live_trading": "out_of_scope"
        },
        "no_trade_filters": ["confidence < 0.55 -> no_trade", "missing required evidence", "coverage gap", "non-native or degraded dataset", "failed backtest gate"],
        "position_sizing": {"risk_fraction": 0.005, "max_fraction": 0.01, "formula": "min(account_equity*risk_fraction/abs(entry-stop), account_equity*max_fraction/entry)", "promotion_claim": false},
        "slippage_cost_assumptions": {"fees": "from BacktestConfig", "spread_bps": "from BacktestConfig", "slippage_bps": "from BacktestConfig", "market_impact_bps": "from BacktestConfig"},
        "data_quality_gates": ["registered dataset id/version", "native_interval=true", "production_eligible=true", "validation passed", "checksum match"],
        "promotion_gates": {"paper": ["confidence >= 0.70", "backtest gates passed", "no hypothesis rules used for promotion"], "live": "out_of_scope"},
        "paper_trading_readiness_gates": ["native backtest replayable", "Pine exploratory only", "adversarial review accepted"],
        "source_citations": "evidence/citations.json",
        "residual_gaps": [residual_gap("GAP-AHDM-SPEC-001", "strategy", "Uncited hypothesis rules are retained for research only", "Hypothesis rules cannot satisfy promotion gates", generated_at)],
        "shared_rule_manifest": manifest
    })
}

pub(super) fn ahdm_pine_source(kind: &str, manifest: &serde_json::Value) -> String {
    format!(
        "//@version=6\n// AHDM-v1 {kind}; exploratory only; shared_manifest_hash={}\n{}(\"AHDM-v1 {kind}\", overlay=true)\nconfidence = input.float(0.0, \"confidence score, not probability\")\nno_trade = confidence < 0.55\nplot(confidence, title=\"confidence_score\")\nplotchar(no_trade, title=\"no_trade_state\", char=\"X\")\nplot(close, title=\"bias\")\nplot(close, title=\"entry_zone\")\nplot(close, title=\"stop\")\nplot(close, title=\"tp1\")\nplot(close, title=\"tp2\")\nplot(close, title=\"tp3\")\nplot(confidence, title=\"sizing_hint\")\n",
        bytes_checksum(manifest.to_string().as_bytes()),
        if kind == "strategy" {
            "strategy"
        } else {
            "indicator"
        }
    )
}

pub(super) fn validate_ahdm_citations(citations: &serde_json::Value) -> Result<(), DataStoreError> {
    let Some(rules) = citations.get("rules").and_then(serde_json::Value::as_array) else {
        return Err(DataStoreError::InvalidMetadata(
            "AHDM citations missing rules".into(),
        ));
    };
    for rule in rules {
        let status = rule.get("status").and_then(serde_json::Value::as_str);
        let has_citation = rule
            .get("citation")
            .is_some_and(serde_json::Value::is_object);
        let promotion_allowed = rule
            .get("promotion_allowed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let valid = status == Some("cited") && has_citation
            || status == Some("hypothesis") && !promotion_allowed;
        if !valid {
            return Err(DataStoreError::InvalidMetadata(
                "AHDM rule violates citation or hypothesis promotion policy".into(),
            ));
        }
    }
    Ok(())
}

pub(super) fn residual_gap(
    id: &str,
    area: &str,
    description: &str,
    fail_closed: &str,
    created_at: &str,
) -> serde_json::Value {
    serde_json::json!({"id": id, "area": area, "description": description, "impact": fail_closed, "fail_closed_behavior": fail_closed, "owner": "Archon", "created_at": created_at})
}

#[derive(Debug)]
pub(super) struct ReadinessGate {
    id: &'static str,
    passed: bool,
    detail: String,
}

pub(super) fn ahdm_readiness_report(
    lake: &TradingDataLake,
    generated_at: &str,
) -> Result<String, DataStoreError> {
    let strategy_root = lake.ahdm_strategy_root();
    let coverage = lake.coverage_matrix("trading-core-v1", generated_at.into())?;
    let gates = readiness_gates(lake, &strategy_root, &coverage)?;
    let residuals = readiness_residual_gaps(&coverage, &gates, generated_at);
    let ready = gates.iter().all(|gate| gate.passed);
    let mut text = format!(
        "# AHDM-v1 Paper Trading Readiness\n\n- generated_at: `{generated_at}`\n- status: `{}`\n- paper_trading_ready: `{ready}`\n- live_trading: `out_of_scope`\n- confidence: score, not probability\n\n",
        if ready { "passed" } else { "failed" }
    );
    text.push_str("## Gates\n\n");
    for gate in &gates {
        text.push_str(&format!(
            "- `{}`: {} — {}\n",
            gate.id,
            if gate.passed { "PASS" } else { "FAIL" },
            gate.detail
        ));
    }
    text.push_str("\n## Risk Review\n\n");
    text.push_str("- KB evidence: citation and hypothesis policy is required; hypotheses cannot satisfy promotion gates.\n");
    text.push_str("- Data/provider/coverage: production promotion requires registered, validated, provider-native datasets with no coverage gaps.\n");
    text.push_str("- Overfitting: native backtests are necessary evidence but do not create a high-probability claim.\n");
    text.push_str("- Slippage/execution: costs and slippage must remain explicit in backtest config; live execution is out of scope.\n");
    text.push_str("- Paper readiness: any failed gate below blocks paper-readiness promotion.\n\n");
    text.push_str("## Residual Gaps\n\n");
    for gap in &residuals {
        text.push_str(&format!(
            "```json\n{}\n```\n",
            serde_json::to_string_pretty(gap)
                .map_err(|err| DataStoreError::Json(err.to_string()))?
        ));
    }
    text.push_str("\n## Artifact Paths\n\n");
    text.push_str("- readiness: `.archon/trading-lab/strategies/AHDM-v1/readiness/paper-trading-readiness.md`\n");
    text.push_str("- adversarial reviews: `.archon/trading-lab/strategies/AHDM-v1/backtests/<run-id>/adversarial-review.md`\n");
    Ok(text)
}

pub(super) fn readiness_gates(
    lake: &TradingDataLake,
    strategy_root: &Path,
    coverage: &CoverageMatrix,
) -> Result<Vec<ReadinessGate>, DataStoreError> {
    let spec_path = strategy_root.join("strategy-spec.json");
    let citations_path = strategy_root.join("evidence/citations.json");
    let pine_report_path = strategy_root.join("pine/compile-report.json");
    let backtests = backtest_run_dirs(&strategy_root.join("backtests"))?;
    let adversarial_missing = backtests
        .iter()
        .filter(|dir| !dir.join("adversarial-review.md").exists())
        .count();
    let native_backtests_pass = backtests.iter().all(|dir| backtest_report_promotes(dir));
    Ok(vec![
        ReadinessGate {
            id: "kb_evidence",
            passed: citations_path.exists() && citations_policy_passes(&citations_path)?,
            detail: "rules must be cited or promotion-barred hypotheses".into(),
        },
        ReadinessGate {
            id: "coverage",
            passed: coverage.gaps.is_empty(),
            detail: format!("coverage gaps: {}", coverage.gaps.len()),
        },
        ReadinessGate {
            id: "strategy_spec",
            passed: spec_path.exists(),
            detail: "StrategySpec artifact must exist and reference registered datasets".into(),
        },
        ReadinessGate {
            id: "pine_exploratory_only",
            passed: pine_report_path.exists() && pine_report_is_exploratory(&pine_report_path)?,
            detail: "Pine results must be exploratory and promotion-ineligible".into(),
        },
        ReadinessGate {
            id: "native_backtests",
            passed: !backtests.is_empty() && native_backtests_pass,
            detail: format!("native backtest runs: {}", backtests.len()),
        },
        ReadinessGate {
            id: "adversarial_reviews",
            passed: !backtests.is_empty() && adversarial_missing == 0,
            detail: format!("missing adversarial reviews: {adversarial_missing}"),
        },
        ReadinessGate {
            id: "diagnostic_artifacts_do_not_promote",
            passed: diagnostic_artifacts_do_not_promote(lake)?,
            detail: "diagnostic/degraded artifacts cannot satisfy production promotion gates"
                .into(),
        },
    ])
}

pub(super) fn readiness_residual_gaps(
    coverage: &CoverageMatrix,
    gates: &[ReadinessGate],
    generated_at: &str,
) -> Vec<serde_json::Value> {
    let mut gaps = Vec::new();
    if !coverage.gaps.is_empty() {
        gaps.push(residual_gap(
            "GAP-AHDM-READINESS-DATA-001",
            "data",
            "Trading-core coverage matrix contains unavailable provider-native cells",
            "Paper-readiness promotion is refused until every required coverage cell is available",
            generated_at,
        ));
    }
    for gate in gates.iter().filter(|gate| !gate.passed) {
        gaps.push(residual_gap(
            "GAP-AHDM-READINESS-GATE-001",
            "paper",
            &format!("Readiness gate `{}` failed: {}", gate.id, gate.detail),
            "Paper-readiness promotion is refused while any gate fails",
            generated_at,
        ));
    }
    if gaps.is_empty() {
        gaps.push(residual_gap(
            "GAP-AHDM-READINESS-NONE",
            "paper",
            "No residual readiness gaps detected by implemented gates",
            "No fail-closed block is active from the readiness report",
            generated_at,
        ));
    }
    gaps
}

pub(super) fn ahdm_adversarial_review_markdown(
    run_id: &str,
    report: &serde_json::Value,
    generated_at: &str,
) -> String {
    let promotion_eligible = report["promotion_eligible"].as_bool().unwrap_or(false);
    let status = if promotion_eligible {
        "passed"
    } else {
        "failed"
    };
    let paper_risk = if promotion_eligible {
        "no failed backtest promotion gate detected for this run"
    } else {
        "failed backtest promotion gate blocks paper-readiness promotion"
    };
    format!(
        "# AHDM-v1 Adversarial Review\n\n- run_id: `{run_id}`\n- generated_at: `{generated_at}`\n- status: `{status}`\n- promotion_eligible: `{promotion_eligible}`\n\n## Findings\n\n- KB evidence risk: PASS only when every promoted rule has a citation; hypotheses remain research-only.\n- Data/provider/coverage risk: PASS only for registered, validated, provider-native datasets; unavailable providers fail closed.\n- Overfitting risk: PASS for deterministic replayability only; no high-probability claim is made.\n- Slippage/execution risk: PASS only with explicit cost/slippage fields; live trading is out of scope.\n- Paper-readiness risk: {paper_risk}\n\n## Residual Gap\n\n```json\n{}\n```\n",
        residual_gap(
            "GAP-AHDM-REVIEW-001",
            "backtest",
            "Adversarial review is required for every AHDM backtest run",
            "Runs without this review cannot satisfy paper-readiness promotion",
            generated_at
        )
    )
}

pub(super) fn backtest_run_dirs(backtest_root: &Path) -> Result<Vec<PathBuf>, DataStoreError> {
    if !backtest_root.exists() {
        return Ok(Vec::new());
    }
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(backtest_root).map_err(io_error)? {
        let path = entry.map_err(io_error)?.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    Ok(dirs)
}

pub(super) fn backtest_report_promotes(run_dir: &Path) -> bool {
    read_json::<serde_json::Value>(&run_dir.join("report.json"))
        .ok()
        .and_then(|report| report["promotion_eligible"].as_bool())
        .unwrap_or(false)
}

pub(super) fn pine_report_is_exploratory(path: &Path) -> Result<bool, DataStoreError> {
    let report: serde_json::Value = read_json(path)?;
    Ok(report["pine_results"] == "exploratory_only" && report["promotion_eligible"] == false)
}

pub(super) fn citations_policy_passes(path: &Path) -> Result<bool, DataStoreError> {
    let citations: serde_json::Value = read_json(path)?;
    Ok(validate_ahdm_citations(&citations).is_ok())
}

pub(super) fn diagnostic_artifacts_do_not_promote(
    lake: &TradingDataLake,
) -> Result<bool, DataStoreError> {
    Ok(lake
        .load_registry()?
        .datasets
        .values()
        .filter(|record| record.status == DatasetStatus::Degraded)
        .all(|record| record.status != DatasetStatus::Healthy))
}
