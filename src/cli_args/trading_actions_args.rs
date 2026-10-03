//! Argument structs for `TradingCliAction` variants. Each struct gets its own
//! clap `augment_args` frame, so the frames do not stack (#233).

use clap::Args;

use super::*;

#[derive(Args, Debug, Clone, PartialEq)]
pub struct TradingCliSetupArgs {
    /// Project root to configure (default: current directory)
    #[arg(long)]
    pub target: Option<PathBuf>,
    /// Check readiness only; do not clone or install
    #[arg(long)]
    pub check: bool,
    /// Skip TradingView MCP clone/npm install
    #[arg(long)]
    pub skip_tradingview: bool,
    /// Skip OpenBB virtualenv install
    #[arg(long)]
    pub skip_openbb: bool,
}

#[derive(Args, Debug, Clone, PartialEq)]
pub struct TradingCliDispatchArgs {
    /// Trading command family to route
    #[arg(value_enum)]
    pub command: TradingCliCommand,
    /// Action to authorize for the command family
    #[arg(long, value_enum)]
    pub action: TradingCliVerb,
    /// Persona requesting the action
    #[arg(long, value_enum, default_value = "per07-observer")]
    pub persona: TradingCliPersona,
    /// Assert maker-checker approval for actions that require it
    #[arg(long)]
    pub maker_checker_approved: bool,
    /// Enable live-policy gate for this dry dispatch check
    #[arg(long)]
    pub live_policy_enabled: bool,
}

#[derive(Args, Debug, Clone, PartialEq)]
pub struct TradingCliKillArgs {
    /// Operator or system actor requesting the halt
    #[arg(long)]
    pub actor: String,
    /// Human-readable halt reason
    #[arg(long)]
    pub reason: String,
    /// Number of working orders expected to be cancelled
    #[arg(long, default_value_t = 0)]
    pub working_orders: usize,
}
