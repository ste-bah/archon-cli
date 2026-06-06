use clap::{Subcommand, ValueEnum};
use std::path::PathBuf;

use super::trading_market_actions::{
    TradingCliBacktestAction, TradingCliDataAction, TradingCliOhlcvFormat,
};

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum TradingCliAction {
    /// Show Trading Lab command readiness and live-trading safety state
    Status,
    /// List command routes into the Trading Lab libraries
    Routes,
    /// Install or check external Trading Lab tools for a project
    Setup {
        /// Project root to configure (default: current directory)
        #[arg(long)]
        target: Option<PathBuf>,
        /// Check readiness only; do not clone or install
        #[arg(long)]
        check: bool,
        /// Skip TradingView MCP clone/npm install
        #[arg(long)]
        skip_tradingview: bool,
        /// Skip OpenBB virtualenv install
        #[arg(long)]
        skip_openbb: bool,
    },
    /// Inspect configured external tool readiness
    Tools {
        #[command(subcommand)]
        action: TradingCliToolsAction,
    },
    /// Run TradingView MCP/CLI helper commands
    Tv {
        #[command(subcommand)]
        action: TradingCliTvAction,
    },
    /// Generate and validate Pine Script prototypes
    Pine {
        #[command(subcommand)]
        action: TradingCliPineAction,
    },
    /// Validate and hash Trading Lab StrategySpec files
    Spec {
        #[command(subcommand)]
        action: TradingCliSpecAction,
    },
    /// Run deterministic native Trading Lab backtests
    Backtest {
        #[command(subcommand)]
        action: TradingCliBacktestAction,
    },
    /// Ingest, list, inspect, and export persistent market datasets
    Data {
        #[command(subcommand)]
        action: TradingCliDataAction,
    },
    /// Exercise paper-trading order and sample gates
    Paper {
        #[command(subcommand)]
        action: TradingCliPaperAction,
    },
    /// Generate Trading Lab workflow specs for end-to-end strategy lifecycles
    Workflow {
        #[command(subcommand)]
        action: TradingCliWorkflowAction,
    },
    /// Inspect OpenBB local API/runtime readiness
    Openbb {
        #[command(subcommand)]
        action: TradingCliOpenBbAction,
    },
    /// Evaluate promotion evidence gates
    Promote {
        #[command(subcommand)]
        action: TradingCliPromoteAction,
    },
    /// Evaluate live-readiness gates without submitting broker orders
    Live {
        #[command(subcommand)]
        action: TradingCliLiveAction,
    },
    /// Exercise the fenced trading command dispatcher without placing orders
    Dispatch {
        /// Trading command family to route
        #[arg(value_enum)]
        command: TradingCliCommand,
        /// Action to authorize for the command family
        #[arg(long, value_enum)]
        action: TradingCliVerb,
        /// Persona requesting the action
        #[arg(long, value_enum, default_value = "per07-observer")]
        persona: TradingCliPersona,
        /// Assert maker-checker approval for actions that require it
        #[arg(long)]
        maker_checker_approved: bool,
        /// Enable live-policy gate for this dry dispatch check
        #[arg(long)]
        live_policy_enabled: bool,
    },
    /// Trigger the out-of-band Trading Lab kill-switch path
    Kill {
        /// Operator or system actor requesting the halt
        #[arg(long)]
        actor: String,
        /// Human-readable halt reason
        #[arg(long)]
        reason: String,
        /// Number of working orders expected to be cancelled
        #[arg(long, default_value_t = 0)]
        working_orders: usize,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TradingCliToolsAction {
    /// Show local TradingView MCP and OpenBB setup status
    Status {
        /// Project root to inspect (default: current directory)
        #[arg(long)]
        target: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TradingCliTvAction {
    /// Run the TradingView MCP CLI status check (`tv status`)
    Status {
        /// Project root containing .archon/tools/tradingview-mcp
        #[arg(long)]
        target: Option<PathBuf>,
    },
    /// Launch TradingView Desktop with CDP enabled via the pinned helper
    Launch {
        /// Project root containing .archon/tools/tradingview-mcp
        #[arg(long)]
        target: Option<PathBuf>,
        /// CDP port for TradingView Desktop
        #[arg(long, default_value_t = 9222)]
        port: u16,
    },
    /// Pass arguments directly to the installed `tv` CLI
    Cli {
        /// Project root containing .archon/tools/tradingview-mcp
        #[arg(long)]
        target: Option<PathBuf>,
        /// Arguments after `tv`; for example: `pine analyze --file script.pine`
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TradingCliPineAction {
    /// Generate Pine v6 indicator/strategy files from a StrategySpec JSON file
    Generate {
        /// Stable strategy id used in generated script names
        #[arg(long)]
        strategy_id: String,
        /// StrategySpec JSON file
        #[arg(long)]
        spec: PathBuf,
        /// Output directory for generated .pine files and manifest.json
        #[arg(long)]
        out: PathBuf,
    },
    /// Run TradingView MCP offline Pine static analysis on a source file
    Analyze {
        /// Project root containing .archon/tools/tradingview-mcp
        #[arg(long)]
        target: Option<PathBuf>,
        /// Pine source file
        #[arg(long)]
        source: PathBuf,
    },
    /// Run TradingView MCP server-side Pine compile check on a source file
    Check {
        /// Project root containing .archon/tools/tradingview-mcp
        #[arg(long)]
        target: Option<PathBuf>,
        /// Pine source file
        #[arg(long)]
        source: PathBuf,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TradingCliSpecAction {
    /// Validate a StrategySpec JSON file and print its content hash
    Validate {
        /// StrategySpec JSON file
        #[arg(long)]
        spec: PathBuf,
        /// Optional JSON output path for the validation report
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TradingCliPaperAction {
    /// Submit a paper OrderIntent through the Risk Governor and ledger path
    Submit {
        /// OrderIntent JSON file
        #[arg(long)]
        intent: PathBuf,
        /// Optional AccountState JSON file; defaults are used when omitted
        #[arg(long)]
        account: Option<PathBuf>,
        /// Optional MarketState JSON file; defaults are used when omitted
        #[arg(long)]
        market: Option<PathBuf>,
        /// Optional audit ledger JSONL path
        #[arg(long)]
        audit: Option<PathBuf>,
        /// Optional JSON output path for the paper result
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Evaluate the paper sample gate from PaperSample JSON
    Sample {
        /// PaperSample JSON file
        #[arg(long)]
        sample: PathBuf,
        /// Optional JSON output path for the sample-gate report
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Risk-gate a paper order, then submit a TradingView replay trade
    TradingviewReplaySubmit {
        /// Project root containing .archon/tools/tradingview-mcp
        #[arg(long)]
        target: Option<PathBuf>,
        /// OrderIntent JSON file; must be Paper + Market
        #[arg(long)]
        intent: PathBuf,
        /// Optional AccountState JSON file; defaults are used when omitted
        #[arg(long)]
        account: Option<PathBuf>,
        /// Optional MarketState JSON file; defaults are used when omitted
        #[arg(long)]
        market: Option<PathBuf>,
        /// Optional audit ledger JSONL path
        #[arg(long)]
        audit: Option<PathBuf>,
        /// Pinned adapter identity, for example `tradesdontlie@abcdef1`
        #[arg(long)]
        adapter_pin: String,
        /// Assert that the TradingView write tier is enabled by policy
        #[arg(long)]
        write_tier_enabled: bool,
        /// Assert that this adapter/version is sandbox-certified
        #[arg(long)]
        sandbox_certified: bool,
        /// Maker-checker request id
        #[arg(long)]
        approval_id: String,
        /// Maker actor id
        #[arg(long)]
        maker: String,
        /// Checker actor id; must differ from maker
        #[arg(long)]
        checker: String,
        /// Approval rationale
        #[arg(long)]
        rationale: String,
        /// Optional JSON output path for the combined paper/TradingView report
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TradingCliWorkflowAction {
    /// Write a provider-neutral end-to-end Trading Lab workflow spec
    Plan {
        /// Strategy idea or research goal
        #[arg(long)]
        idea: String,
        /// Repository or project root that workflow implementation stages may edit
        #[arg(long)]
        repository: PathBuf,
        /// Optional PRD path to anchor requirements
        #[arg(long)]
        prd: Option<PathBuf>,
        /// Optional decomposed task directory to anchor implementation
        #[arg(long)]
        tasks: Option<PathBuf>,
        /// Trading KB names to consult
        #[arg(long = "kb")]
        kb: Vec<String>,
        /// Include a TradingView replay paper-submit stage
        #[arg(long)]
        tradingview_replay: bool,
        /// Output workflow YAML path
        #[arg(long)]
        out: PathBuf,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TradingCliOpenBbAction {
    /// Show OpenBB venv/API command readiness
    Status {
        /// Project root containing .archon/tools/openbb-venv
        #[arg(long)]
        target: Option<PathBuf>,
    },
    /// Fetch through the local OpenBB API and persist a governed dataset report
    Fetch {
        /// OpenBbRequest JSON file
        #[arg(long)]
        request: PathBuf,
        /// Dataset metadata map JSON required by the lake registry
        #[arg(long)]
        metadata: PathBuf,
        /// DataQuality JSON required to prevent unsafe promotion bypass
        #[arg(long)]
        quality: PathBuf,
        /// Access mode for fail-closed cache behavior
        #[arg(long, value_enum, default_value = "research")]
        mode: TradingCliOpenBbMode,
        /// Project root containing OpenBB tooling
        #[arg(long)]
        target: Option<PathBuf>,
        /// Optional JSON output path for the governed dataset report
        #[arg(long)]
        out: Option<PathBuf>,
        /// Store an OHLCV response directly in the persistent Trading Lab data lake
        #[arg(long)]
        store_ohlcv: bool,
        /// Response format to parse when --store-ohlcv is set
        #[arg(long, value_enum, default_value = "json")]
        response_format: TradingCliOhlcvFormat,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq, Eq)]
pub enum TradingCliPromoteAction {
    /// Evaluate a one-step promotion gate from StrategySpec and evidence JSON
    Check {
        /// StrategySpec JSON file
        #[arg(long)]
        spec: PathBuf,
        /// Target promotion status
        #[arg(long, value_enum)]
        target: TradingCliPromotionStatus,
        /// JSON array of PromotionEvidence records
        #[arg(long)]
        evidence: PathBuf,
        /// Optional PaperSample JSON for Paper -> LivePilot
        #[arg(long)]
        paper_sample: Option<PathBuf>,
        /// Optional SessionPostmortem JSON for Paper -> LivePilot
        #[arg(long)]
        postmortem: Option<PathBuf>,
        /// Optional JSON output path for the promotion report
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug, Clone, PartialEq)]
pub enum TradingCliLiveAction {
    /// Evaluate full live enablement JSON; no broker order is submitted
    EnableCheck {
        /// LiveEnablementRequest JSON file
        #[arg(long)]
        request: PathBuf,
        /// Optional JSON output path for the decision
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Build and validate a bounded live-pilot plan
    Pilot {
        /// Strategy id for the pilot
        #[arg(long)]
        strategy_id: String,
        /// Account equity in USD
        #[arg(long)]
        account_equity: f64,
        /// Requested pilot capital in USD
        #[arg(long)]
        requested_capital: f64,
        /// Optional RiskPolicy JSON file; defaults are used when omitted
        #[arg(long)]
        policy: Option<PathBuf>,
        /// Optional JSON output path for the pilot plan
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Evaluate Phase-5 autonomy prerequisites; no policy is changed
    Phase5Check {
        /// StrategySpec JSON file
        #[arg(long)]
        spec: PathBuf,
        /// Phase5Evidence JSON file
        #[arg(long)]
        evidence: PathBuf,
        /// Optional RiskPolicy JSON file; defaults are used when omitted
        #[arg(long)]
        policy: Option<PathBuf>,
        /// Optional JSON output path for the decision
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingCliOpenBbMode {
    Research,
    LiveRequired,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingCliPromotionStatus {
    Idea,
    Research,
    Backtest,
    Paper,
    LivePilot,
    Retired,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingCliCommand {
    Kb,
    Spec,
    Pine,
    Backtest,
    Paper,
    Live,
    Promote,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingCliVerb {
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

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradingCliPersona {
    Per01HumanGovernor,
    Per05ExecutionAgent,
    Per07Observer,
}
