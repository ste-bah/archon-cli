//! Argument structs for `Commands` variants. Each struct gets its own
//! clap `augment_args` frame, so the frames do not stack (#233).

use clap::Args;

use super::*;

#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Check for updates without downloading
    #[arg(long)]
    pub check: bool,
    /// Install even if already at latest version
    #[arg(long)]
    pub force: bool,
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Port to listen on
    #[arg(long, default_value = "8420")]
    pub port: u16,
    /// Path to load or store the access token
    #[arg(long)]
    pub token_path: Option<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct AcpArgs {
    /// Project root the agent works in. Defaults to the directory the
    /// editor spawned the process in.
    #[arg(long)]
    pub workspace: Option<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct IdeStdioArgs {
    /// Project root the agent works in. Defaults to the directory the
    /// IDE spawned the process in.
    #[arg(long)]
    pub workspace: Option<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct WebArgs {
    /// Port to listen on (default from config: 8421)
    #[arg(long)]
    pub port: Option<u16>,
    /// Address to bind to (default from config: 127.0.0.1)
    #[arg(long)]
    pub bind_address: Option<String>,
    /// Do not open browser automatically
    #[arg(long)]
    pub no_open: bool,
    /// UNSAFE: allow a non-localhost bind without bearer-token auth
    #[arg(long)]
    pub allow_unauthenticated_nonlocal_bind: bool,
}

#[derive(Args, Debug)]
pub struct RunAgentAsyncArgs {
    /// Agent name to run
    pub name: String,
    /// Path to input file (use `-` for stdin)
    #[arg(long)]
    pub input: Option<String>,
    /// Agent version constraint
    #[arg(long)]
    pub version: Option<String>,
    /// Detach after submission (don't wait for result)
    #[arg(long)]
    pub detach: bool,
}

#[derive(Args, Debug)]
pub struct DraftArgs {
    /// Path to the context pack JSON
    pub pack: std::path::PathBuf,
    /// Working directory for artifacts + provenance chain
    pub workdir: std::path::PathBuf,
    /// Model override (default: configured Anthropic Opus, else claude-opus-4-8)
    #[arg(long)]
    pub model: Option<String>,
    /// Gate config JSON (default: the pack's p2_style_target.gate_config_path)
    #[arg(long)]
    pub gate_config: Option<std::path::PathBuf>,
}

#[derive(Args, Debug)]
pub struct TaskStatusArgs {
    /// Task ID (UUID)
    pub task_id: String,
    /// Poll every 500ms until terminal state
    #[arg(long)]
    pub watch: bool,
}

#[derive(Args, Debug)]
pub struct TaskResultArgs {
    /// Task ID (UUID)
    pub task_id: String,
    /// Stream result chunks
    #[arg(long)]
    pub stream: bool,
}

#[derive(Args, Debug)]
pub struct TaskCancelArgs {
    /// Task ID (UUID)
    pub task_id: String,
}

#[derive(Args, Debug)]
pub struct TaskListArgs {
    /// Filter by state (Pending, Running, Finished, Failed, Cancelled)
    #[arg(long)]
    pub state: Option<String>,
    /// Filter by agent name
    #[arg(long)]
    pub agent: Option<String>,
    /// Filter tasks created after duration (e.g. "1h", "30m")
    #[arg(long)]
    pub since: Option<String>,
}

#[derive(Args, Debug)]
pub struct TaskEventsArgs {
    /// Task ID (UUID)
    pub task_id: String,
    /// Start from this sequence number
    #[arg(long, default_value = "0")]
    pub from_seq: u64,
}

#[derive(Args, Debug)]
pub struct AgentListArgs {
    /// Include invalid/broken agent entries
    #[arg(long)]
    pub include_invalid: bool,
}

#[derive(Args, Debug)]
pub struct AgentSearchArgs {
    /// Filter by tag (repeatable)
    #[arg(long = "tag", value_name = "TAG")]
    pub tags: Vec<String>,
    /// Filter by capability (repeatable)
    #[arg(long = "capability", value_name = "CAP")]
    pub capabilities: Vec<String>,
    /// Filter by name pattern (glob, e.g. "code-*")
    #[arg(long, value_name = "PATTERN")]
    pub name_pattern: Option<String>,
    /// Filter by version requirement (e.g. "^1", "=2.0.0")
    #[arg(long, value_name = "REQ")]
    pub version: Option<String>,
    /// Filter logic: and (default) or or
    #[arg(long, default_value = "and")]
    pub logic: String,
    /// Include invalid/broken agent entries
    #[arg(long)]
    pub include_invalid: bool,
    /// Remote registry URL to include
    #[arg(long, value_name = "URL")]
    pub registry_url: Option<String>,
}

#[derive(Args, Debug)]
pub struct AgentInfoArgs {
    /// Agent name
    pub name: String,
    /// Pin to a specific version (e.g. "=1.0.1", "^2")
    #[arg(long, value_name = "REQ")]
    pub version: Option<String>,
    /// Output as JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct GametheoryArgs {
    /// PRD shorthand: `archon gametheory "<situation>"`
    pub situation: Option<String>,
    /// PRD shorthand: `archon gametheory --classify-only "<situation>"`
    #[arg(long)]
    pub classify_only: bool,
    /// Bind the run to an ingested document/knowledge pack
    #[arg(long, value_name = "PACK")]
    pub kb: Option<String>,
    /// Path to gametheory spec YAML (searches known locations if omitted)
    #[arg(long, value_name = "PATH")]
    pub spec_path: Option<String>,
    /// Print per-agent gametheory memory recall counts
    #[arg(long)]
    pub debug_memory: bool,
    /// Stop specialist execution when estimated model spend reaches this USD cap
    #[arg(long, default_value_t = 20.0)]
    pub budget: f64,
    /// Maximum specialist concurrency requested for this run
    #[arg(long, default_value_t = 4)]
    pub max_concurrent: usize,
    /// Report style: executive, academic, or technical
    #[arg(long, default_value = "executive")]
    pub style: String,
    /// Enable Tier 11 specialists when policy.gametheory.enable_tier11 also allows it
    #[arg(long)]
    pub enable_tier11: bool,
    #[command(subcommand)]
    pub action: Option<GametheoryAction>,
}
