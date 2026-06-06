use anyhow::{Result, anyhow};
use archon_trading::spec_registry::{StrategySpec, parse_strategy_spec_json};
use serde::Serialize;

use crate::cli_args::TradingCliSpecAction;
use crate::command::trading_io::write_or_render;

#[derive(Debug, Serialize)]
struct SpecValidationReport {
    valid: bool,
    missing_or_invalid: Vec<&'static str>,
    content_hash: Option<String>,
}

pub(crate) fn render_spec(action: &TradingCliSpecAction) -> Result<String> {
    match action {
        TradingCliSpecAction::Validate { spec, out } => {
            let text = std::fs::read_to_string(spec)?;
            let spec_value = parse_strategy_spec_json(&text)
                .map_err(|err| anyhow!("invalid StrategySpec JSON: {err:?}"))?;
            let report = validation_report(&spec_value);
            write_or_render(&report, out.as_deref())
        }
    }
}

fn validation_report(spec: &StrategySpec) -> SpecValidationReport {
    let missing_or_invalid = spec.validate();
    let content_hash = if missing_or_invalid.is_empty() {
        spec.content_hash().ok()
    } else {
        None
    };
    SpecValidationReport {
        valid: missing_or_invalid.is_empty(),
        missing_or_invalid,
        content_hash,
    }
}
