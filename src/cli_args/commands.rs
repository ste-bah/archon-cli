use clap::Subcommand;

use super::{
    AuthArgs, BehaviourAction, ChatArgs, CompletionAction, ConstellationAction, DocsAction,
    GametheoryAction, KbAction, LearningAction, MeaningAction, MemoryAction, PipelineAction,
    PluginAction, ProvAction, ProvidersAction, RemoteAction, SelfAction, TeamAction,
};

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Authenticate with Anthropic via OAuth PKCE flow (deprecated alias for `auth login`)
    Login,
    /// Manage provider authentication
    Auth(AuthArgs),
    /// Single-turn chat completion against a selected provider
    Chat(ChatArgs),
    /// Inspect provider registry and Archon-level capability support
    Providers {
        #[command(subcommand)]
        action: Option<ProvidersAction>,
    },
    /// Manage plugins
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
    /// Check for and install updates
    Update {
        /// Check for updates without downloading
        #[arg(long)]
        check: bool,
        /// Install even if already at latest version
        #[arg(long)]
        force: bool,
    },
    /// Remote agent mode
    Remote {
        #[command(subcommand)]
        action: RemoteAction,
    },
    /// Start a WebSocket server for remote agent access
    Serve {
        /// Port to listen on
        #[arg(long, default_value = "8420")]
        port: u16,
        /// Path to load or store the access token
        #[arg(long)]
        token_path: Option<std::path::PathBuf>,
    },
    /// Manage and run multi-agent teams
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },
    /// Run in IDE stdio mode (JSON-RPC over stdin/stdout)
    IdeStdio,
    /// Run and manage multi-agent pipelines
    Pipeline {
        #[command(subcommand)]
        action: PipelineAction,
    },
    /// Start the browser-based web UI on localhost
    Web {
        /// Port to listen on (default from config: 8421)
        #[arg(long)]
        port: Option<u16>,
        /// Address to bind to (default from config: 127.0.0.1)
        #[arg(long)]
        bind_address: Option<String>,
        /// Do not open browser automatically
        #[arg(long)]
        no_open: bool,
    },
    /// Submit an async agent task
    RunAgentAsync {
        /// Agent name to run
        name: String,
        /// Path to input file (use `-` for stdin)
        #[arg(long)]
        input: Option<String>,
        /// Agent version constraint
        #[arg(long)]
        version: Option<String>,
        /// Detach after submission (don't wait for result)
        #[arg(long)]
        detach: bool,
    },
    /// Manage governed learning behaviour
    Behaviour {
        #[command(subcommand)]
        action: BehaviourAction,
    },
    /// Inspect learning subsystem diagnostics
    Learning {
        #[command(subcommand)]
        action: LearningAction,
    },
    /// Check status of an async task
    TaskStatus {
        /// Task ID (UUID)
        task_id: String,
        /// Poll every 500ms until terminal state
        #[arg(long)]
        watch: bool,
    },
    /// Get result of a completed async task
    TaskResult {
        /// Task ID (UUID)
        task_id: String,
        /// Stream result chunks
        #[arg(long)]
        stream: bool,
    },
    /// Cancel a running async task
    TaskCancel {
        /// Task ID (UUID)
        task_id: String,
    },
    /// List async tasks
    TaskList {
        /// Filter by state (Pending, Running, Finished, Failed, Cancelled)
        #[arg(long)]
        state: Option<String>,
        /// Filter by agent name
        #[arg(long)]
        agent: Option<String>,
        /// Filter tasks created after duration (e.g. "1h", "30m")
        #[arg(long)]
        since: Option<String>,
    },
    /// Stream events for a task (NDJSON)
    TaskEvents {
        /// Task ID (UUID)
        task_id: String,
        /// Start from this sequence number
        #[arg(long, default_value = "0")]
        from_seq: u64,
    },
    /// Show task execution metrics (prometheus format)
    Metrics,
    /// List all discovered agents
    AgentList {
        /// Include invalid/broken agent entries
        #[arg(long)]
        include_invalid: bool,
    },
    /// Search agents by tag, capability, name pattern, or version
    AgentSearch {
        /// Filter by tag (repeatable)
        #[arg(long = "tag", value_name = "TAG")]
        tags: Vec<String>,
        /// Filter by capability (repeatable)
        #[arg(long = "capability", value_name = "CAP")]
        capabilities: Vec<String>,
        /// Filter by name pattern (glob, e.g. "code-*")
        #[arg(long, value_name = "PATTERN")]
        name_pattern: Option<String>,
        /// Filter by version requirement (e.g. "^1", "=2.0.0")
        #[arg(long, value_name = "REQ")]
        version: Option<String>,
        /// Filter logic: and (default) or or
        #[arg(long, default_value = "and")]
        logic: String,
        /// Include invalid/broken agent entries
        #[arg(long)]
        include_invalid: bool,
        /// Remote registry URL to include
        #[arg(long, value_name = "URL")]
        registry_url: Option<String>,
    },
    /// Show detailed information about a specific agent
    AgentInfo {
        /// Agent name
        name: String,
        /// Pin to a specific version (e.g. "=1.0.1", "^2")
        #[arg(long, value_name = "REQ")]
        version: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Manage the knowledge base
    Kb {
        #[command(subcommand)]
        action: KbAction,
    },
    /// Manage document ingestion, inspection, and status
    Docs {
        #[command(subcommand)]
        action: DocsAction,
    },
    /// Inspect and export provenance traces
    Prov {
        #[command(subcommand)]
        action: ProvAction,
    },
    /// Build meaning samples, pairs, triplets, and eval data
    Meaning {
        #[command(subcommand)]
        action: MeaningAction,
    },
    /// Build and inspect learned constellation centroids
    Constellation {
        #[command(subcommand)]
        action: ConstellationAction,
    },
    /// Manage the persistent memory graph
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
    /// Inspect Archon's self-calibration records
    #[command(name = "self")]
    SelfCmd {
        #[command(subcommand)]
        action: SelfAction,
    },
    /// Game-theory strategic analysis
    Gametheory {
        /// PRD shorthand: `archon gametheory "<situation>"`
        situation: Option<String>,
        /// PRD shorthand: `archon gametheory --classify-only "<situation>"`
        #[arg(long)]
        classify_only: bool,
        /// Bind the run to an ingested document/knowledge pack
        #[arg(long, value_name = "PACK")]
        kb: Option<String>,
        /// Path to gametheory spec YAML (searches known locations if omitted)
        #[arg(long, value_name = "PATH")]
        spec_path: Option<String>,
        /// Print per-agent gametheory memory recall counts
        #[arg(long)]
        debug_memory: bool,
        /// Stop specialist execution when estimated model spend reaches this USD cap
        #[arg(long, default_value_t = 20.0)]
        budget: f64,
        /// Maximum specialist concurrency requested for this run
        #[arg(long, default_value_t = 4)]
        max_concurrent: usize,
        /// Report style: executive, academic, or technical
        #[arg(long, default_value = "executive")]
        style: String,
        /// Enable Tier 11 specialists when policy.gametheory.enable_tier11 also allows it
        #[arg(long)]
        enable_tier11: bool,
        #[command(subcommand)]
        action: Option<GametheoryAction>,
    },
    /// Completion-integrity checks (TSPEC §10)
    Completion {
        #[command(subcommand)]
        action: CompletionAction,
    },
}
