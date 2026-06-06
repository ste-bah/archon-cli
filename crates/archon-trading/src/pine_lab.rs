use crate::TradingError;
use crate::adapters::tv_mcp::{TradingViewMcpAdapter, TvMcpError, TvMcpTransport};
use crate::spec_registry::{PromotionStatus, StrategySpec};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const PINE_VERSION: &str = "//@version=6";
const OMIT_NO_TRADABLE_RULES: &str = "OMIT_NO_TRADABLE_RULES";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptVariant {
    Indicator,
    Strategy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertHandoff {
    None,
    OrderIntent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PineInput {
    pub name: String,
    pub value: String,
    pub group: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedPineScript {
    pub source_strategy_id: String,
    pub symbol: String,
    pub variant: ScriptVariant,
    pub source: String,
    pub alert_handoff: AlertHandoff,
    pub inputs: Vec<PineInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioPineRecord {
    pub source_strategy_id: String,
    pub symbols: Vec<String>,
    pub script_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PineGenerationReport {
    pub scripts: Vec<GeneratedPineScript>,
    pub portfolio_record: Option<PortfolioPineRecord>,
    pub audit_events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisteredPineScript {
    pub script_id: String,
    pub source_strategy_id: String,
    pub author_agent: String,
    pub review_status: String,
    pub compile_status: String,
    pub content_hash: String,
    pub script: GeneratedPineScript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PineCompileProof {
    source_hash: String,
    docs_checked_before_code: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PineLabError {
    SpecInvalid(Vec<&'static str>),
    UnapprovedStatus,
    CrossSymbolLogic,
    PineV5Forbidden,
    CompileFailed,
    TvMcp(TvMcpError),
}

#[derive(Debug, Default, Clone)]
pub struct PineScriptRegistry {
    records: BTreeMap<String, RegisteredPineScript>,
}

impl PineScriptRegistry {
    pub fn register_compiled(
        &mut self,
        script: GeneratedPineScript,
        author_agent: impl Into<String>,
        review_status: impl Into<String>,
        proof: PineCompileProof,
    ) -> Result<RegisteredPineScript, PineLabError> {
        reject_v5(&script.source)?;
        if !proof.accepts(&script) {
            return Err(PineLabError::CompileFailed);
        }
        let content_hash = blake3::hash(script.source.as_bytes()).to_hex().to_string();
        let script_id = script_id(&script.source_strategy_id, &script.symbol, script.variant);
        let record = RegisteredPineScript {
            script_id: script_id.clone(),
            source_strategy_id: script.source_strategy_id.clone(),
            author_agent: author_agent.into(),
            review_status: review_status.into(),
            compile_status: "compiled".into(),
            content_hash,
            script,
        };
        self.records.insert(script_id, record.clone());
        Ok(record)
    }

    pub fn get(&self, script_id: &str) -> Option<&RegisteredPineScript> {
        self.records.get(script_id)
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }
}

impl PineCompileProof {
    pub(crate) fn from_compiled_script(script: &GeneratedPineScript) -> Self {
        Self {
            source_hash: blake3::hash(script.source.as_bytes()).to_hex().to_string(),
            docs_checked_before_code: true,
        }
    }

    fn accepts(&self, script: &GeneratedPineScript) -> bool {
        self.docs_checked_before_code
            && self.source_hash == blake3::hash(script.source.as_bytes()).to_hex().to_string()
    }
}

pub fn generate_pine_scripts(
    strategy_id: &str,
    spec: &StrategySpec,
) -> Result<PineGenerationReport, PineLabError> {
    let invalid = spec.validate();
    if !invalid.is_empty() {
        return Err(PineLabError::SpecInvalid(invalid));
    }
    if spec.pine_omission_reason() == Some(OMIT_NO_TRADABLE_RULES) {
        return Ok(PineGenerationReport {
            scripts: Vec::new(),
            portfolio_record: None,
            audit_events: vec![OMIT_NO_TRADABLE_RULES.into()],
        });
    }
    ensure_research_or_later(spec)?;
    let symbols = instrument_symbols(spec)?;
    let scripts = build_symbol_scripts(strategy_id, spec, &symbols)?;
    Ok(PineGenerationReport {
        portfolio_record: portfolio_record(strategy_id, &symbols, &scripts),
        scripts,
        audit_events: Vec::new(),
    })
}

pub fn compile_and_register<T: TvMcpTransport>(
    registry: &mut PineScriptRegistry,
    adapter: &TradingViewMcpAdapter,
    transport: &mut T,
    script: GeneratedPineScript,
    author_agent: impl Into<String>,
    review_status: impl Into<String>,
) -> Result<RegisteredPineScript, PineLabError> {
    reject_v5(&script.source)?;
    adapter
        .pine_compile_check(transport, &script.source)
        .map_err(PineLabError::TvMcp)?;
    let proof = PineCompileProof::from_compiled_script(&script);
    registry.register_compiled(script, author_agent, review_status, proof)
}

pub fn pine_alert_to_non_authoritative_intent(
    script: &GeneratedPineScript,
) -> Result<serde_json::Value, TradingError> {
    if script.alert_handoff != AlertHandoff::OrderIntent {
        return Err(TradingError::PolicyDenied);
    }
    Ok(serde_json::json!({
        "source": "pine_alert_non_authoritative",
        "strategy_id": script.source_strategy_id,
        "symbol": script.symbol,
        "requires_risk_governor": true
    }))
}

fn ensure_research_or_later(spec: &StrategySpec) -> Result<(), PineLabError> {
    match spec.spec_f15_promotion_status {
        Some(PromotionStatus::Research)
        | Some(PromotionStatus::Backtest)
        | Some(PromotionStatus::Paper)
        | Some(PromotionStatus::LivePilot) => Ok(()),
        _ => Err(PineLabError::UnapprovedStatus),
    }
}

fn instrument_symbols(spec: &StrategySpec) -> Result<Vec<String>, PineLabError> {
    spec.spec_f01_instrument_universe
        .as_ref()
        .ok_or_else(|| PineLabError::SpecInvalid(vec!["SPEC-F01"]))
        .map(|instruments| instruments.iter().map(|item| item.symbol.clone()).collect())
}

fn build_symbol_scripts(
    strategy_id: &str,
    spec: &StrategySpec,
    symbols: &[String],
) -> Result<Vec<GeneratedPineScript>, PineLabError> {
    let mut scripts = Vec::new();
    for symbol in symbols {
        scripts.push(build_script(
            strategy_id,
            spec,
            symbol,
            ScriptVariant::Indicator,
        )?);
        scripts.push(build_script(
            strategy_id,
            spec,
            symbol,
            ScriptVariant::Strategy,
        )?);
    }
    Ok(scripts)
}

fn build_script(
    strategy_id: &str,
    spec: &StrategySpec,
    symbol: &str,
    variant: ScriptVariant,
) -> Result<GeneratedPineScript, PineLabError> {
    let formulas = spec
        .spec_f06_indicator_formulas
        .as_ref()
        .ok_or_else(|| PineLabError::SpecInvalid(vec!["SPEC-F06"]))?;
    let rules = spec
        .spec_f05_entry_exit_rules
        .as_ref()
        .ok_or_else(|| PineLabError::SpecInvalid(vec!["SPEC-F05"]))?;
    reject_cross_symbol_rules(&rules.rules)?;
    let inputs = configurable_inputs(spec);
    let source = render_script(
        strategy_id,
        symbol,
        variant,
        &formulas.formulas,
        &rules.rules,
        &inputs,
    );
    reject_v5(&source)?;
    Ok(GeneratedPineScript {
        source_strategy_id: strategy_id.into(),
        symbol: symbol.into(),
        variant,
        source,
        alert_handoff: alert_handoff(variant),
        inputs,
    })
}

fn render_script(
    strategy_id: &str,
    symbol: &str,
    variant: ScriptVariant,
    formulas: &[String],
    rules: &[String],
    inputs: &[PineInput],
) -> String {
    let declaration = match variant {
        ScriptVariant::Indicator => format!("indicator(\"{strategy_id} {symbol}\", overlay=true)"),
        ScriptVariant::Strategy => format!("strategy(\"{strategy_id} {symbol}\", overlay=true)"),
    };
    let input_lines = inputs
        .iter()
        .map(render_input)
        .collect::<Vec<_>>()
        .join("\n");
    let formula_notes = formulas.join(" | ");
    let rule_notes = rules.join(" | ");
    let alert_line = match variant {
        ScriptVariant::Indicator => "// alert_handoff=none".to_string(),
        ScriptVariant::Strategy => format!(
            "alertcondition(true, title=\"{strategy_id} intent\", message=\"{{\\\"strategy_id\\\":\\\"{strategy_id}\\\",\\\"symbol\\\":\\\"{symbol}\\\"}}\")"
        ),
    };
    format!(
        "{PINE_VERSION}\n{declaration}\n{input_lines}\n// SPEC-F06: {formula_notes}\n// SPEC-F05: {rule_notes}\nplot(close, title=\"close\")\n{alert_line}\n"
    )
}

fn render_input(input: &PineInput) -> String {
    format!(
        "input.string(\"{}\", title=\"{}\", group=\"{}\")",
        input.value, input.name, input.group
    )
}

fn configurable_inputs(spec: &StrategySpec) -> Vec<PineInput> {
    let mut inputs = vec![pine_input("display", "close", "display")];
    if let Some(time) = &spec.spec_f02_timeframe_session {
        inputs.push(pine_input("session", &time.session_hours, "sessions"));
    }
    if let Some(stops) = &spec.spec_f08_stops {
        inputs.push(pine_input(
            "max_drawdown_pct",
            &stops.max_strategy_drawdown_pct.to_string(),
            "risk",
        ));
    }
    inputs.push(pine_input("threshold_window", "20", "thresholds_windows"));
    inputs
}

fn pine_input(name: &str, value: &str, group: &str) -> PineInput {
    PineInput {
        name: name.into(),
        value: value.into(),
        group: group.into(),
    }
}

fn portfolio_record(
    strategy_id: &str,
    symbols: &[String],
    scripts: &[GeneratedPineScript],
) -> Option<PortfolioPineRecord> {
    if symbols.len() <= 1 {
        return None;
    }
    Some(PortfolioPineRecord {
        source_strategy_id: strategy_id.into(),
        symbols: symbols.to_vec(),
        script_ids: scripts
            .iter()
            .map(|script| script_id(strategy_id, &script.symbol, script.variant))
            .collect(),
    })
}

fn reject_cross_symbol_rules(rules: &[String]) -> Result<(), PineLabError> {
    let cross_symbol = rules.iter().any(|rule| {
        let lower = rule.to_ascii_lowercase();
        lower.contains("request.security") || lower.contains("cross-symbol")
    });
    if cross_symbol {
        Err(PineLabError::CrossSymbolLogic)
    } else {
        Ok(())
    }
}

fn reject_v5(source: &str) -> Result<(), PineLabError> {
    if source.contains("//@version=5") || !source.contains(PINE_VERSION) {
        return Err(PineLabError::PineV5Forbidden);
    }
    Ok(())
}

fn script_id(strategy_id: &str, symbol: &str, variant: ScriptVariant) -> String {
    format!("pine:{strategy_id}:{symbol}:{}", short_variant(variant))
}

const fn alert_handoff(variant: ScriptVariant) -> AlertHandoff {
    match variant {
        ScriptVariant::Indicator => AlertHandoff::None,
        ScriptVariant::Strategy => AlertHandoff::OrderIntent,
    }
}

const fn short_variant(variant: ScriptVariant) -> &'static str {
    match variant {
        ScriptVariant::Indicator => "indicator",
        ScriptVariant::Strategy => "strategy",
    }
}

#[cfg(test)]
#[path = "pine_lab_tests.rs"]
mod tests;
