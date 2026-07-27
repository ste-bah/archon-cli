use serde_json::json;

pub(super) fn ahdm_rule_manifest(generated_at: &str) -> serde_json::Value {
    json!({
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
            liquidity_sweep_reversal(),
            trend_continuation_pullback(),
            range_mean_reversion()
        ],
        "thresholds": {
            "no_trade_below_confidence": 0.55,
            "paper_consideration_min_confidence": 0.70,
            "paper_requires_backtest_gates": true
        },
        "native_and_pine_parity_key": "AHDM-v1/shared-rule-manifest"
    })
}

fn cited_rule(id: &str, weight: u64, kb: &str) -> serde_json::Value {
    let citation = citation_for_rule(id, kb);
    json!({
        "id": id,
        "weight": weight,
        "status": "cited",
        "citation": citation,
        "source_excerpt": citation["excerpt"].clone(),
        "promotion_allowed": true,
        "required_evidence": ["registered_native_dataset", "kb_citation"]
    })
}

fn citation_for_rule(id: &str, kb: &str) -> serde_json::Value {
    let (locator, excerpt) = match id {
        "higher_timeframe_trend_regime" => (
            "trading-hybrid-system:prioritize-higher-timeframe-context",
            "Use higher-timeframe market context as the first filter before lower-timeframe execution signals.",
        ),
        "liquidity_location_prior_highs_lows" => (
            "trading-market-structure:liquidity-prior-high-low-sweeps",
            "Prior highs and lows are liquidity locations; sweeps into those levels require confirmation before acting.",
        ),
        "vector_volume_behavior" => (
            "trading-market-structure:volume-confirms-structure-displacement",
            "Treat volume behavior as confirmation for displacement or exhaustion rather than as a standalone entry signal.",
        ),
        "adr_awr_range_state" => (
            "trading-strategy-research:daily-weekly-range-state-filter",
            "Daily and weekly range state should constrain trade selection when extension or compression changes expected follow-through.",
        ),
        "session_timing_macro_filter" => (
            "trading-execution:session-timing-and-news-risk-filter",
            "Execution rules must account for session timing and macro-event risk before opening a position.",
        ),
        "recent_postmortem_penalty" => (
            "trading-postmortems:recent-mistake-penalty-feedback",
            "Recent postmortem findings should reduce confidence when the setup resembles a documented failure mode.",
        ),
        _ => (
            "unknown:missing-locator",
            "No reviewed source excerpt is available.",
        ),
    };
    json!({
        "kb": kb,
        "locator": locator,
        "excerpt": excerpt,
        "reviewed_source_excerpt": true
    })
}

fn hypothesis_rule(id: &str, weight: u64) -> serde_json::Value {
    json!({
        "id": id,
        "weight": weight,
        "status": "hypothesis",
        "citation": serde_json::Value::Null,
        "source_excerpt": serde_json::Value::Null,
        "promotion_allowed": false,
        "required_evidence": ["registered_native_dataset", "kb_citation_or_hypothesis_label"]
    })
}

fn entry_model(
    id: &str,
    entry_zone: &str,
    invalidation: &str,
    stop: &str,
    targets: [&str; 3],
    filters: [&str; 3],
) -> serde_json::Value {
    json!({
        "id": id,
        "entry_zone": entry_zone,
        "required_evidence": ["registered_native_dataset", "bias_component_score", "invalidation_level"],
        "evidence_requirements": ["registered_native_dataset", "bias_component_score", "invalidation_level"],
        "fail_closed_no_trade": true,
        "missing_evidence_behavior": "no_trade_fail_closed",
        "invalidation": invalidation,
        "stop": stop,
        "tp1": targets[0],
        "tp2": targets[1],
        "tp3": targets[2],
        "filters": filters,
        "sizing": {
            "risk_fraction": 0.005,
            "max_fraction": 0.01,
            "formula": "min(account_equity*risk_fraction/abs(entry-stop), account_equity*max_fraction/entry)",
            "invalid_inputs": "no_trade_fail_closed"
        },
        "outputs": ["entry_zone", "stop", "tp1", "tp2", "tp3", "position_size"]
    })
}

fn liquidity_sweep_reversal() -> serde_json::Value {
    entry_model(
        "liquidity_sweep_reversal",
        "confirmed sweep of prior high/low liquidity with vector-volume exhaustion and reclaim/acceptance back inside the swept level",
        "sweep extreme is reclaimed in the wrong direction or confidence/data gates fail",
        "beyond the sweep extreme/invalidation level",
        [
            "return to nearest intraday structure or VWAP",
            "opposing liquidity pool or prior session midpoint",
            "higher-timeframe objective or ADR/AWR constrained target",
        ],
        [
            "requires liquidity_location_prior_highs_lows evidence",
            "requires vector_volume_behavior confirmation",
            "blocked by session_timing_macro_filter risk",
        ],
    )
}

fn trend_continuation_pullback() -> serde_json::Value {
    entry_model(
        "trend_continuation_pullback",
        "higher-timeframe trend/regime alignment followed by pullback into VWAP/EMA or structure support/resistance with continuation confirmation",
        "trend regime breaks, pullback level fails, or confidence/data gates fail",
        "beyond pullback structure or trend-continuation invalidation level",
        [
            "prior impulse high/low retest",
            "next liquidity objective in trend direction",
            "ADR/AWR constrained extension objective",
        ],
        [
            "requires higher_timeframe_trend_regime alignment",
            "requires vwap_ema_relationship evidence",
            "blocked when adr_awr_range_state shows exhausted extension",
        ],
    )
}

fn range_mean_reversion() -> serde_json::Value {
    entry_model(
        "range_mean_reversion",
        "range-state confirmation with rejection at range extreme and target back toward VWAP/midrange",
        "range breaks into accepted trend, range extreme fails, or confidence/data gates fail",
        "outside accepted range boundary or setup invalidation level",
        [
            "range midpoint or VWAP",
            "opposite side of value area",
            "opposing range extreme only when ADR/AWR state permits",
        ],
        [
            "requires adr_awr_range_state range context",
            "requires liquidity_location_prior_highs_lows boundary evidence",
            "blocked by higher_timeframe_trend_regime breakout acceptance",
        ],
    )
}
