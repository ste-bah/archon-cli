use anyhow::{Result, anyhow};
use archon_tools::trading::kill::{
    OutOfBandKillRequest, render_kill_command_status, trigger_out_of_band_kill,
};
use archon_tools::trading::{
    TradingAction, TradingCommand, TradingConfig, TradingPersona, TradingRequest, command_route,
    dispatch_cli,
};

use crate::cli_args::{TradingCliAction, TradingCliCommand, TradingCliPersona, TradingCliVerb};

pub(crate) fn handle_trading_command(action: &TradingCliAction) -> Result<()> {
    println!("{}", render_trading_command(action)?);
    Ok(())
}

pub(crate) fn render_trading_command(action: &TradingCliAction) -> Result<String> {
    match action {
        TradingCliAction::Status => Ok(render_status()),
        TradingCliAction::Routes => Ok(render_routes()),
        TradingCliAction::Setup {
            target,
            check,
            skip_tradingview,
            skip_openbb,
        } => crate::command::trading_tools::run_setup_script(
            target.as_ref(),
            *check,
            *skip_tradingview,
            *skip_openbb,
        ),
        TradingCliAction::Tools { action } => match action {
            crate::cli_args::TradingCliToolsAction::Status { target } => {
                crate::command::trading_tools::render_tools_status(target.as_ref())
            }
        },
        TradingCliAction::Tv { action } => crate::command::trading_tv::render_tv(action),
        TradingCliAction::Pine { action } => crate::command::trading_pine::render_pine(action),
        TradingCliAction::Spec { action } => crate::command::trading_spec::render_spec(action),
        TradingCliAction::Backtest { action } => {
            crate::command::trading_backtest::render_backtest(action)
        }
        TradingCliAction::Data { action } => crate::command::trading_data::render_data(action),
        TradingCliAction::Paper { action } => crate::command::trading_paper::render_paper(action),
        TradingCliAction::Openbb { action } => {
            crate::command::trading_openbb::render_openbb(action)
        }
        TradingCliAction::Workflow { action } => {
            crate::command::trading_workflow::render_workflow(action)
        }
        TradingCliAction::Promote { action } => {
            crate::command::trading_promote::render_promote(action)
        }
        TradingCliAction::Live { action } => crate::command::trading_live::render_live(action),
        TradingCliAction::Dispatch {
            command,
            action,
            persona,
            maker_checker_approved,
            live_policy_enabled,
        } => render_dispatch(
            *command,
            *action,
            *persona,
            *maker_checker_approved,
            *live_policy_enabled,
        ),
        TradingCliAction::Kill {
            actor,
            reason,
            working_orders,
        } => render_kill(actor, reason, *working_orders),
    }
}

fn render_status() -> String {
    [
        "Trading Lab status",
        "  core crate: implemented",
        "  command surface: archon trading / /trading",
        "  live trading: disabled by default and policy gated",
        "  broker execution: never submitted by default; live checks are explicit gates",
        "  TradingView MCP: supported via project .mcp.json and tv CLI",
        "  OpenBB: supported via governed local API fetches and project .archon/tools/openbb-venv",
        "",
        "Useful commands:",
        "  archon trading routes",
        "  archon trading setup --target /path/to/project",
        "  archon trading tools status --target /path/to/project",
        "  archon trading spec validate --spec strategy-spec.json",
        "  archon trading data ingest-ohlcv --source candles.csv --format csv --dataset-id btc-1d --version v1 --provider openbb --symbol BTCUSD",
        "  archon trading data list",
        "  archon trading backtest run --config backtest.json --fills fills.json",
        "  archon trading backtest run-ohlcv --config backtest.json --dataset-id btc-1d --version v1 --quantity 1",
        "  archon trading paper submit --intent order-intent.json",
        "  archon trading paper tradingview-replay-submit --intent order-intent.json --adapter-pin tradesdontlie@abcdef1 --write-tier-enabled --sandbox-certified --approval-id r1 --maker alice --checker bob --rationale \"approved\"",
        "  archon trading workflow plan --idea \"BTC Elliott Wave strategy\" --repository /path/to/repo --tasks /path/to/tasks --out trading-workflow.yaml",
        "  archon trading promote check --spec strategy-spec.json --target paper --evidence evidence.json",
        "  archon trading live enable-check --request live-enable.json",
        "  archon trading tv status --target /path/to/project",
        "  archon trading pine generate --strategy-id demo --spec spec.json --out ./pine",
        "  archon trading openbb status --target /path/to/project",
        "  archon trading openbb fetch --request request.json --metadata metadata.json --quality quality.json",
        "  archon trading dispatch backtest --action run-backtest --persona per05-execution-agent",
        "  archon trading dispatch live --action submit-live-order --persona per01-human-governor --maker-checker-approved --live-policy-enabled",
        "  archon trading kill --actor operator --reason \"manual halt\" --working-orders 0",
    ]
    .join("\n")
}

fn render_routes() -> String {
    let mut out = String::from("Trading Lab command routes\n");
    for command in [
        TradingCommand::Kb,
        TradingCommand::Spec,
        TradingCommand::Pine,
        TradingCommand::Backtest,
        TradingCommand::Paper,
        TradingCommand::Live,
        TradingCommand::Promote,
    ] {
        out.push_str(&format!("  {command:?} -> {}\n", command_route(command)));
    }
    out.push_str("  Kill -> archon_tools::trading::kill\n");
    out
}

fn render_dispatch(
    command: TradingCliCommand,
    action: TradingCliVerb,
    persona: TradingCliPersona,
    maker_checker_approved: bool,
    live_policy_enabled: bool,
) -> Result<String> {
    let config = TradingConfig {
        trading_enabled: true,
        live_policy_enabled,
    };
    let request = TradingRequest {
        command: command.into(),
        action: action.into(),
        persona: persona.into(),
        maker_checker_approved,
    };
    let result =
        dispatch_cli(request, &config).map_err(|err| anyhow!("trading dispatch refused: {err}"))?;

    Ok(format!(
        "Trading dry-dispatch accepted; no broker order submitted\n  command: {:?}\n  action: {:?}\n  persona: {:?}\n  route: {}\n  reason: {}\n  live_policy_enabled: {}",
        result.command,
        result.action,
        persona,
        result.library_route,
        result.reason,
        live_policy_enabled
    ))
}

fn render_kill(actor: &str, reason: &str, working_orders: usize) -> Result<String> {
    let response = trigger_out_of_band_kill(OutOfBandKillRequest {
        actor: actor.to_string(),
        reason: reason.to_string(),
        working_orders,
    })
    .map_err(|err| anyhow!("trading kill refused: {err}"))?;
    Ok(format!(
        "{}\n  nfr_002_met: {}\n  ui_required: {}",
        render_kill_command_status(&response),
        response.receipt.meets_nfr_002(),
        response.ui_required
    ))
}

impl From<TradingCliCommand> for TradingCommand {
    fn from(value: TradingCliCommand) -> Self {
        match value {
            TradingCliCommand::Kb => Self::Kb,
            TradingCliCommand::Spec => Self::Spec,
            TradingCliCommand::Pine => Self::Pine,
            TradingCliCommand::Backtest => Self::Backtest,
            TradingCliCommand::Paper => Self::Paper,
            TradingCliCommand::Live => Self::Live,
            TradingCliCommand::Promote => Self::Promote,
        }
    }
}

impl From<TradingCliVerb> for TradingAction {
    fn from(value: TradingCliVerb) -> Self {
        match value {
            TradingCliVerb::Read => Self::Read,
            TradingCliVerb::WriteKb => Self::WriteKb,
            TradingCliVerb::WriteRisk => Self::WriteRisk,
            TradingCliVerb::DraftSpec => Self::DraftSpec,
            TradingCliVerb::GeneratePine => Self::GeneratePine,
            TradingCliVerb::RunBacktest => Self::RunBacktest,
            TradingCliVerb::SubmitPaperOrder => Self::SubmitPaperOrder,
            TradingCliVerb::SubmitLiveOrder => Self::SubmitLiveOrder,
            TradingCliVerb::Promote => Self::Promote,
        }
    }
}

impl From<TradingCliPersona> for TradingPersona {
    fn from(value: TradingCliPersona) -> Self {
        match value {
            TradingCliPersona::Per01HumanGovernor => Self::Per01HumanGovernor,
            TradingCliPersona::Per05ExecutionAgent => Self::Per05ExecutionAgent,
            TradingCliPersona::Per07Observer => Self::Per07Observer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_names_safe_command_surface() {
        let text = render_trading_command(&TradingCliAction::Status).expect("status renders");
        assert!(text.contains("command surface"));
        assert!(text.contains("disabled by default"));
        assert!(text.contains("TradingView MCP"));
    }

    #[test]
    fn backtest_dispatch_accepts_execution_agent() {
        let text = render_trading_command(&TradingCliAction::Dispatch {
            command: TradingCliCommand::Backtest,
            action: TradingCliVerb::RunBacktest,
            persona: TradingCliPersona::Per05ExecutionAgent,
            maker_checker_approved: false,
            live_policy_enabled: false,
        })
        .expect("backtest dispatch accepted");

        assert!(text.contains("Trading dry-dispatch accepted"));
        assert!(text.contains("archon_trading::backtest"));
    }

    #[test]
    fn observer_write_dispatch_is_rejected() {
        let err = render_trading_command(&TradingCliAction::Dispatch {
            command: TradingCliCommand::Kb,
            action: TradingCliVerb::WriteKb,
            persona: TradingCliPersona::Per07Observer,
            maker_checker_approved: false,
            live_policy_enabled: false,
        })
        .expect_err("observer write must be rejected");

        assert!(err.to_string().contains("PER-07"));
    }

    #[test]
    fn live_dispatch_is_policy_gated() {
        let err = render_trading_command(&TradingCliAction::Dispatch {
            command: TradingCliCommand::Live,
            action: TradingCliVerb::SubmitLiveOrder,
            persona: TradingCliPersona::Per01HumanGovernor,
            maker_checker_approved: true,
            live_policy_enabled: false,
        })
        .expect_err("live without policy is rejected");

        assert!(err.to_string().contains("live trading refused"));
    }

    #[test]
    fn kill_command_uses_out_of_band_path() {
        let text = render_trading_command(&TradingCliAction::Kill {
            actor: "operator".to_string(),
            reason: "manual halt".to_string(),
            working_orders: 1,
        })
        .expect("kill command succeeds");

        assert!(text.contains("halted=true"));
        assert!(text.contains("nfr_002_met: true"));
    }
}
