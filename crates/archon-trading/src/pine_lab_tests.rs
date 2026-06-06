use super::*;
use crate::adapters::tv_mcp::{TimedMcpResult, TvMcpConfig};
use crate::spec_registry::*;
use archon_mcp::types::{McpToolResult, ToolContent};
use serde_json::Value;
use std::time::Duration;

struct CompileOkTransport;

impl TvMcpTransport for CompileOkTransport {
    fn call_tool(&mut self, _tool_name: &str, _arguments: Value) -> Result<TimedMcpResult, String> {
        Ok(TimedMcpResult {
            result: McpToolResult {
                content: vec![ToolContent::Text {
                    text: "compiled".into(),
                }],
                is_error: false,
            },
            elapsed: Duration::from_millis(25),
        })
    }
}

fn full_spec(symbols: Vec<&str>, status: PromotionStatus) -> StrategySpec {
    StrategySpec {
        spec_f01_instrument_universe: Some(
            symbols
                .into_iter()
                .map(|symbol| Instrument {
                    symbol: symbol.into(),
                    venue: "ARCA".into(),
                    asset_class: "equity".into(),
                })
                .collect(),
        ),
        spec_f02_timeframe_session: Some(TimeSession {
            timeframe: "1D".into(),
            session_hours: "regular".into(),
        }),
        spec_f03_market_regime_assumptions: Some(vec!["baseline".into()]),
        spec_f04_data_dependencies: Some(vec![DatasetRef {
            dataset_id: "ohlcv".into(),
            version: "v1".into(),
        }]),
        spec_f05_entry_exit_rules: Some(RuleSet {
            rules: vec!["close > sma20".into()],
        }),
        spec_f06_indicator_formulas: Some(FormulaSet {
            formulas: vec!["ta.sma(close, 20)".into()],
        }),
        spec_f07_position_sizing: Some(PositionSizing {
            model: "fixed_fractional".into(),
            max_risk_pct: "1".into(),
        }),
        spec_f08_stops: Some(SpecF08Stops {
            stop_rules: vec!["2atr".into()],
            take_profit_rules: vec!["3atr".into()],
            trailing_rules: vec![],
            max_strategy_drawdown_pct: 8.0,
        }),
        spec_f09_invalidation_rules: Some(RuleSet {
            rules: vec!["regime shift".into()],
        }),
        spec_f10_no_trade_conditions: Some(RuleSet {
            rules: vec!["event risk".into()],
        }),
        spec_f11_cost_assumptions: Some(CostModel {
            slippage_bps: 2,
            fee_bps: 1,
        }),
        spec_f12_benchmark: Some(BenchmarkRef {
            symbol: "SPY".into(),
            source: "approved".into(),
        }),
        spec_f13_expected_failure_modes: Some(vec!["gap".into()]),
        spec_f14_data_quality_tolerances_ms: Some(BTreeMap::from([("ohlcv".into(), 5000)])),
        spec_f15_promotion_status: Some(status),
    }
}

#[test]
fn t_pine_01_generates_v6_indicator_and_strategy_after_research_approval() {
    let report = generate_pine_scripts(
        "strat-a",
        &full_spec(vec!["SPY"], PromotionStatus::Research),
    )
    .unwrap();
    assert_eq!(report.scripts.len(), 2);
    assert!(
        report
            .scripts
            .iter()
            .all(|script| script.source.contains("//@version=6"))
    );
    assert_eq!(report.scripts[0].alert_handoff, AlertHandoff::None);
    assert_eq!(report.scripts[1].alert_handoff, AlertHandoff::OrderIntent);
}

#[test]
fn t_pine_02_multi_symbol_decomposes_without_cross_symbol_logic() {
    let report = generate_pine_scripts(
        "strat-b",
        &full_spec(vec!["SPY", "QQQ"], PromotionStatus::Backtest),
    )
    .unwrap();
    assert_eq!(report.scripts.len(), 4);
    assert_eq!(report.portfolio_record.unwrap().symbols, vec!["SPY", "QQQ"]);
    let mut spec = full_spec(vec!["SPY"], PromotionStatus::Research);
    spec.spec_f05_entry_exit_rules.as_mut().unwrap().rules =
        vec!["request.security(\"QQQ\", \"D\", close)".into()];
    assert_eq!(
        generate_pine_scripts("bad", &spec),
        Err(PineLabError::CrossSymbolLogic)
    );
}

#[test]
fn t_pine_03_omits_only_empty_tradable_universe_and_logs_reason() {
    let report =
        generate_pine_scripts("strat-c", &full_spec(vec![], PromotionStatus::Research)).unwrap();
    assert!(report.scripts.is_empty());
    assert_eq!(report.audit_events, vec![OMIT_NO_TRADABLE_RULES]);
}

#[test]
fn ac_023_registry_stores_only_compiled_v6_scripts() {
    let script = generate_pine_scripts(
        "strat-d",
        &full_spec(vec!["SPY"], PromotionStatus::Research),
    )
    .unwrap()
    .scripts
    .remove(0);
    let mut registry = PineScriptRegistry::default();
    assert_eq!(
        registry.register_compiled(script.clone(), "agent", "reviewed", false),
        Err(PineLabError::CompileFailed)
    );
    let adapter = TradingViewMcpAdapter::new(TvMcpConfig::pinned("vendor@abcdef1")).unwrap();
    let record = compile_and_register(
        &mut registry,
        &adapter,
        &mut CompileOkTransport,
        script,
        "agent",
        "reviewed",
    )
    .unwrap();
    assert_eq!(record.compile_status, "compiled");
    assert_eq!(registry.len(), 1);
}

#[test]
fn ac_003_alerts_are_non_authoritative_order_intents() {
    let script = generate_pine_scripts(
        "strat-e",
        &full_spec(vec!["SPY"], PromotionStatus::Research),
    )
    .unwrap()
    .scripts
    .remove(1);
    let intent = pine_alert_to_non_authoritative_intent(&script).unwrap();
    assert_eq!(intent["requires_risk_governor"], true);
}
