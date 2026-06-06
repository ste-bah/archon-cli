pub mod kill;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradingPersona {
    Per01HumanGovernor,
    Per05ExecutionAgent,
    Per07Observer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradingCommand {
    Kb,
    Spec,
    Pine,
    Backtest,
    Paper,
    Live,
    Promote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TradingAction {
    Read,
    WriteKb,
    WriteRisk,
    DraftSpec,
    GeneratePine,
    RunBacktest,
    SubmitPaperOrder,
    SubmitLiveOrder,
    Promote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradingConfig {
    pub trading_enabled: bool,
    pub live_policy_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradingRequest {
    pub command: TradingCommand,
    pub action: TradingAction,
    pub persona: TradingPersona,
    pub maker_checker_approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradingDispatchResult {
    pub accepted: bool,
    pub command: TradingCommand,
    pub action: TradingAction,
    pub reason: &'static str,
    pub library_route: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolRequest {
    pub tool_name: String,
    pub command: TradingCommand,
    pub action: TradingAction,
    pub persona: TradingPersona,
    pub maker_checker_approved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentToolResponse {
    pub accepted: bool,
    pub route: &'static str,
    pub reason: &'static str,
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self {
            trading_enabled: true,
            live_policy_enabled: false,
        }
    }
}

pub fn dispatch_cli(
    request: TradingRequest,
    config: &TradingConfig,
) -> Result<TradingDispatchResult, String> {
    let route = command_route(request.command);
    ensure_command_matches_action(request.command, request.action)?;
    enforce_live_guard(request.command, config)?;
    enforce_persona_fence(
        request.persona,
        request.action,
        request.maker_checker_approved,
    )?;
    Ok(TradingDispatchResult {
        accepted: true,
        command: request.command,
        action: request.action,
        reason: "accepted by shared trading command path",
        library_route: route,
    })
}

pub fn dispatch_agent_tool(
    request: AgentToolRequest,
    config: &TradingConfig,
) -> Result<AgentToolResponse, String> {
    if request.tool_name.trim().is_empty() {
        return Err("tool_name is required".to_string());
    }
    let dispatch = dispatch_cli(
        TradingRequest {
            command: request.command,
            action: request.action,
            persona: request.persona,
            maker_checker_approved: request.maker_checker_approved,
        },
        config,
    )?;
    Ok(AgentToolResponse {
        accepted: dispatch.accepted,
        route: dispatch.library_route,
        reason: "accepted by fenced agent-tool handler",
    })
}

pub fn command_route(command: TradingCommand) -> &'static str {
    match command {
        TradingCommand::Kb => "archon_trading::kb",
        TradingCommand::Spec => "archon_trading::spec_registry",
        TradingCommand::Pine => "archon_trading::pine_lab",
        TradingCommand::Backtest => "archon_trading::backtest",
        TradingCommand::Paper => "archon_trading::paper_terminal",
        TradingCommand::Live => "archon_trading::live_enablement",
        TradingCommand::Promote => "archon_trading::promotion",
    }
}

pub fn parse_trading_command(input: &str) -> Result<TradingCommand, String> {
    match input.trim() {
        "kb" => Ok(TradingCommand::Kb),
        "spec" => Ok(TradingCommand::Spec),
        "pine" => Ok(TradingCommand::Pine),
        "backtest" => Ok(TradingCommand::Backtest),
        "paper" => Ok(TradingCommand::Paper),
        "live" => Ok(TradingCommand::Live),
        "promote" => Ok(TradingCommand::Promote),
        "kill" => Err("archon trading kill is owned by TASK-TRL-032".to_string()),
        other => Err(format!("unknown trading command: {other}")),
    }
}

fn enforce_live_guard(command: TradingCommand, config: &TradingConfig) -> Result<(), String> {
    if command == TradingCommand::Live && !(config.trading_enabled && config.live_policy_enabled) {
        return Err("live trading refused: trading.enabled and live policy must both be on".into());
    }
    Ok(())
}

fn enforce_persona_fence(
    persona: TradingPersona,
    action: TradingAction,
    maker_checker_approved: bool,
) -> Result<(), String> {
    if persona == TradingPersona::Per07Observer && action != TradingAction::Read {
        return Err("PER-07 is read-only".to_string());
    }
    if persona == TradingPersona::Per05ExecutionAgent
        && matches!(action, TradingAction::WriteKb | TradingAction::WriteRisk)
    {
        return Err("PER-05 cannot write KB or risk policy".to_string());
    }
    if requires_maker_checker(action) && !maker_checker_approved {
        return Err("maker-checker approval required".to_string());
    }
    if !action_granted(persona, action) {
        return Err("not granted by fail-closed REQ-AGENT-002 matrix".to_string());
    }
    Ok(())
}

fn action_granted(persona: TradingPersona, action: TradingAction) -> bool {
    match persona {
        TradingPersona::Per01HumanGovernor => true,
        TradingPersona::Per05ExecutionAgent => matches!(
            action,
            TradingAction::Read | TradingAction::RunBacktest | TradingAction::SubmitPaperOrder
        ),
        TradingPersona::Per07Observer => action == TradingAction::Read,
    }
}

fn requires_maker_checker(action: TradingAction) -> bool {
    matches!(
        action,
        TradingAction::SubmitLiveOrder | TradingAction::Promote | TradingAction::WriteRisk
    )
}

fn ensure_command_matches_action(
    command: TradingCommand,
    action: TradingAction,
) -> Result<(), String> {
    let valid = match command {
        TradingCommand::Kb => matches!(action, TradingAction::Read | TradingAction::WriteKb),
        TradingCommand::Spec => matches!(action, TradingAction::Read | TradingAction::DraftSpec),
        TradingCommand::Pine => matches!(action, TradingAction::Read | TradingAction::GeneratePine),
        TradingCommand::Backtest => {
            matches!(action, TradingAction::Read | TradingAction::RunBacktest)
        }
        TradingCommand::Paper => matches!(
            action,
            TradingAction::Read | TradingAction::SubmitPaperOrder
        ),
        TradingCommand::Live => {
            matches!(action, TradingAction::Read | TradingAction::SubmitLiveOrder)
        }
        TradingCommand::Promote => matches!(action, TradingAction::Read | TradingAction::Promote),
    };
    valid
        .then_some(())
        .ok_or_else(|| "action is not valid for trading command".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn live_on() -> TradingConfig {
        TradingConfig {
            trading_enabled: true,
            live_policy_enabled: true,
        }
    }

    #[test]
    fn cli_smoke_routes_supported_commands_and_excludes_kill() {
        assert_eq!(parse_trading_command("kb").unwrap(), TradingCommand::Kb);
        assert_eq!(
            parse_trading_command("promote").unwrap(),
            TradingCommand::Promote
        );
        assert!(
            parse_trading_command("kill")
                .unwrap_err()
                .contains("TASK-TRL-032")
        );

        let result = dispatch_cli(
            TradingRequest {
                command: TradingCommand::Backtest,
                action: TradingAction::RunBacktest,
                persona: TradingPersona::Per05ExecutionAgent,
                maker_checker_approved: false,
            },
            &TradingConfig::default(),
        )
        .expect("backtest command is routed");

        assert_eq!(result.library_route, "archon_trading::backtest");
    }

    #[test]
    fn live_subcommands_are_guarded_by_config_and_policy() {
        let request = TradingRequest {
            command: TradingCommand::Live,
            action: TradingAction::Read,
            persona: TradingPersona::Per07Observer,
            maker_checker_approved: false,
        };
        let denied = dispatch_cli(request.clone(), &TradingConfig::default()).unwrap_err();
        assert!(denied.contains("live trading refused"));
        assert!(dispatch_cli(request, &live_on()).is_ok());
    }

    #[test]
    fn t_agent_02_per05_kb_and_risk_writes_are_denied() {
        let kb_error = dispatch_agent_tool(
            AgentToolRequest {
                tool_name: "trading.kb.write".to_string(),
                command: TradingCommand::Kb,
                action: TradingAction::WriteKb,
                persona: TradingPersona::Per05ExecutionAgent,
                maker_checker_approved: true,
            },
            &live_on(),
        )
        .unwrap_err();
        assert!(kb_error.contains("PER-05"));

        let risk_error = enforce_persona_fence(
            TradingPersona::Per05ExecutionAgent,
            TradingAction::WriteRisk,
            true,
        )
        .unwrap_err();
        assert!(risk_error.contains("risk policy"));
    }

    #[test]
    fn t_agent_02_per07_writes_are_denied_fail_closed() {
        let error = dispatch_agent_tool(
            AgentToolRequest {
                tool_name: "trading.promote".to_string(),
                command: TradingCommand::Promote,
                action: TradingAction::Promote,
                persona: TradingPersona::Per07Observer,
                maker_checker_approved: true,
            },
            &live_on(),
        )
        .unwrap_err();
        assert!(error.contains("PER-07"));
    }

    #[test]
    fn shared_dispatch_accepts_human_governor_promote_with_approval() {
        let response = dispatch_agent_tool(
            AgentToolRequest {
                tool_name: "trading.promote".to_string(),
                command: TradingCommand::Promote,
                action: TradingAction::Promote,
                persona: TradingPersona::Per01HumanGovernor,
                maker_checker_approved: true,
            },
            &live_on(),
        )
        .expect("approved promote is routed");

        assert!(response.accepted);
        assert_eq!(response.route, "archon_trading::promotion");
    }
}
