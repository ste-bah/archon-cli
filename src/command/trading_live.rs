use anyhow::Result;
use archon_trading::live_enablement::{LiveEnablementRequest, Phase5Evidence, PilotPlan};
use archon_trading::risk_policy::RiskPolicy;
use archon_trading::spec_registry::StrategySpec;
use serde_json::json;

use crate::cli_args::TradingCliLiveAction;
use crate::command::trading_io::{read_json, write_or_render};

pub(crate) fn render_live(action: &TradingCliLiveAction) -> Result<String> {
    match action {
        TradingCliLiveAction::EnableCheck { request, out } => {
            let request: LiveEnablementRequest = read_json(request, "LiveEnablementRequest")?;
            let report = match request.evaluate() {
                Ok(decision) => json!({"accepted": true, "decision": decision}),
                Err(err) => json!({"accepted": false, "error": format!("{err:?}")}),
            };
            write_or_render(&report, out.as_deref())
        }
        TradingCliLiveAction::Pilot {
            strategy_id,
            account_equity,
            requested_capital,
            policy,
            out,
        } => {
            let policy = read_policy(policy.as_deref())?;
            let report =
                match PilotPlan::new(strategy_id, *account_equity, *requested_capital, &policy) {
                    Ok(plan) => json!({"accepted": true, "pilot_plan": plan}),
                    Err(err) => json!({"accepted": false, "error": format!("{err:?}")}),
                };
            write_or_render(&report, out.as_deref())
        }
        TradingCliLiveAction::Phase5Check {
            spec,
            evidence,
            policy,
            out,
        } => {
            let spec: StrategySpec = read_json(spec, "StrategySpec")?;
            let evidence: Phase5Evidence = read_json(evidence, "Phase5Evidence")?;
            let policy = read_policy(policy.as_deref())?;
            let report = match evidence.evaluate(&spec, &policy) {
                Ok(decision) => json!({"accepted": true, "decision": decision}),
                Err(err) => json!({
                    "accepted": false,
                    "error": format!("{err:?}"),
                    "decision": evidence.blocked_decision(&spec, &policy)
                }),
            };
            write_or_render(&report, out.as_deref())
        }
    }
}

fn read_policy(path: Option<&std::path::Path>) -> Result<RiskPolicy> {
    match path {
        Some(path) => read_json(path, "RiskPolicy"),
        None => Ok(RiskPolicy::default()),
    }
}
