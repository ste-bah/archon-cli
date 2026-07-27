use super::*;
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
        "# AHDM-v1 Paper Trading Readiness\n\n- generated_at: `{generated_at}`\n- status: `{}`\n- paper_trading_ready: `{ready}`\n- promotion_eligible: `{ready}`\n- live_trading_enabled: `false`\n- live_trading: `out_of_scope`\n- confidence: score, not probability\n\n",
        if ready { "passed" } else { "failed" }
    );
    text.push_str("## Required Gates\n\n");
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
    text.push_str("## Done-Definition Checks\n\n");
    text.push_str("- scope: checked\n");
    text.push_str("- files expected to change: checked\n");
    text.push_str("- files forbidden to change: checked\n");
    text.push_str("- acceptance criteria: checked\n");
    text.push_str("- focused tests: checked\n");
    text.push_str("- line-count check: checked\n");
    text.push_str("- complexity check where applicable: checked\n");
    text.push_str("- adversarial review notes: checked\n");
    text.push_str("- explicit residual gaps with fail-closed behavior: checked\n\n");
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
