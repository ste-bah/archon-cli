//! CLI argument definitions for the `archon` binary.
//!
//! Extracted from `main.rs` so the Cli struct can grow without bloating the
//! main module. All clap derive definitions live here.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Archon CLI -- Rust-native AI agent runtime
#[derive(Parser, Debug)]
#[command(name = "archon")]
#[command(version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("ARCHON_GIT_HASH"), ")"))]
#[command(about = "Archon CLI -- Rust-native AI agent runtime", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    // ── Existing flags ─────────────────────────────────────────
    /// Resume a previous session (list recent or specify ID)
    #[arg(long)]
    pub resume: Option<Option<String>>,

    /// Disable auto-resume for this invocation (overrides session.auto_resume=true)
    #[arg(long)]
    pub no_resume: bool,

    /// Enable fast mode (reduced latency, lower quality)
    #[arg(long)]
    pub fast: bool,

    /// Set reasoning effort level (high, medium, low)
    #[arg(long, value_name = "LEVEL")]
    pub effort: Option<String>,

    /// Enable identity spoofing (mimic Claude Code headers)
    #[arg(long)]
    pub identity_spoof: bool,

    /// Remote session URL (for /session slash command QR display).
    /// Sets ARCHON_REMOTE_URL at startup so /session can render the QR.
    /// Format: any URL string (https://…, ws://…, archon://…).
    #[arg(long = "remote-url", value_name = "URL")]
    pub remote_url: Option<String>,

    /// Path to additional TOML settings file (overlay)
    #[arg(long, value_name = "PATH")]
    pub settings: Option<PathBuf>,

    /// Control which config layers to load (comma-separated: user,project,local)
    #[arg(long, value_name = "LAYERS", value_delimiter = ',')]
    pub setting_sources: Option<Vec<String>>,

    // ── Print mode (CLI-218) ───────────────────────────────────
    /// Non-interactive single-query mode (print and exit).
    /// Use `-p "query"` to supply the query inline, or `-p` to read from stdin.
    #[arg(short = 'p', long = "print")]
    pub print: Option<Option<String>>,

    /// Output format for print mode (text, json, stream-json)
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    pub output_format: String,

    /// JSON schema to validate the final assistant output against
    #[arg(long, value_name = "SCHEMA")]
    pub json_schema: Option<String>,

    /// Input format for print mode (text, stream-json)
    #[arg(long, value_name = "FORMAT", default_value = "text")]
    pub input_format: String,

    /// Maximum agentic turns before exit (print mode)
    #[arg(long, value_name = "N")]
    pub max_turns: Option<u32>,

    /// Maximum spending in USD before exit (print mode)
    #[arg(long, value_name = "AMOUNT")]
    pub max_budget_usd: Option<f64>,

    /// Don't persist session to disk (print mode)
    #[arg(long)]
    pub no_session_persistence: bool,

    // ── Session naming & forking (CLI-226) ─────────────────────
    /// Assign a human-readable name to this session
    #[arg(short = 'n', long, value_name = "NAME")]
    pub session_name: Option<String>,

    /// Continue the most recent session in the current directory
    #[arg(short = 'c', long)]
    pub continue_session: bool,

    /// Fork the resumed session instead of appending to it
    #[arg(long)]
    pub fork_session: bool,

    // ── Background sessions (CLI-221) ──────────────────────────
    /// Start a background session. Use `--bg "query"` to supply inline, or `--bg` to read stdin.
    #[arg(long)]
    pub bg: Option<Option<String>>,

    /// Display name for background session
    #[arg(long, value_name = "NAME")]
    pub bg_name: Option<String>,

    /// List background sessions
    #[arg(long)]
    pub ps: bool,

    /// Attach to a running background session (stream logs)
    #[arg(long, value_name = "ID")]
    pub attach: Option<String>,

    /// Kill a background session
    #[arg(long = "kill", value_name = "ID")]
    pub kill_session: Option<String>,

    /// View background session logs (non-streaming)
    #[arg(long, value_name = "ID")]
    pub logs: Option<String>,

    // ── Permissions (CLI-219) ──────────────────────────────────
    /// Permission mode (default, acceptEdits, plan, auto, dontAsk, bypassPermissions)
    #[arg(long, value_name = "MODE")]
    pub permission_mode: Option<String>,

    /// Skip all permission checks (alias for --permission-mode bypassPermissions)
    #[arg(long)]
    pub dangerously_skip_permissions: bool,

    /// Allow bypassPermissions in mode cycle
    #[arg(long)]
    pub allow_dangerously_skip_permissions: bool,

    // ── Session search & management (CLI-208) ──────────────────
    /// Session search and management
    #[arg(long)]
    pub sessions: bool,

    /// Filter sessions by git branch
    #[arg(long, value_name = "BRANCH", requires = "sessions")]
    pub branch: Option<String>,

    /// Filter sessions by directory
    #[arg(long = "dir", value_name = "DIR", requires = "sessions")]
    pub session_dir: Option<String>,

    /// Filter sessions after date (RFC 3339 or YYYY-MM-DD)
    #[arg(long, value_name = "DATE", requires = "sessions")]
    pub after: Option<String>,

    /// Filter sessions before date (RFC 3339 or YYYY-MM-DD)
    #[arg(long, value_name = "DATE", requires = "sessions")]
    pub before: Option<String>,

    /// Full-text search in session messages
    #[arg(long, value_name = "TEXT", requires = "sessions")]
    pub search: Option<String>,

    /// Show session statistics
    #[arg(long, requires = "sessions")]
    pub stats: bool,

    /// Delete a session by ID
    #[arg(long, value_name = "ID", requires = "sessions")]
    pub delete: Option<String>,

    // ── NEW: Model ─────────────────────────────────────────────
    /// Override the default model for this session
    #[arg(long, value_name = "MODEL")]
    pub model: Option<String>,

    // ── NEW: System prompt ─────────────────────────────────────
    /// Replace entire system prompt with this text
    #[arg(long, value_name = "TEXT", conflicts_with = "system_prompt_file")]
    pub system_prompt: Option<String>,

    /// Replace entire system prompt with file contents
    #[arg(long, value_name = "PATH", conflicts_with = "system_prompt")]
    pub system_prompt_file: Option<PathBuf>,

    /// Append text to default system prompt
    #[arg(long, value_name = "TEXT")]
    pub append_system_prompt: Option<String>,

    /// Append file contents to default system prompt
    #[arg(long, value_name = "PATH")]
    pub append_system_prompt_file: Option<PathBuf>,

    // ── NEW: Agent ─────────────────────────────────────────────
    /// Specify agent definition for session
    #[arg(long, value_name = "NAME")]
    pub agent: Option<String>,

    // ── NEW: Configuration ─────────────────────────────────────
    /// Load MCP servers from JSON files (repeatable)
    #[arg(long, value_name = "FILES")]
    pub mcp_config: Vec<PathBuf>,

    /// Only use MCP servers from --mcp-config, ignore discovered ones
    #[arg(long)]
    pub strict_mcp_config: bool,

    /// Add additional working directories for file access
    #[arg(long, value_name = "PATHS")]
    pub add_dir: Vec<PathBuf>,

    // ── NEW: Mode control ──────────────────────────────────────
    /// Minimal mode: skip hooks, ARCHON.md, MCP auto-start
    #[arg(long)]
    pub bare: bool,

    /// Run initialization hooks and start interactive mode
    #[arg(long)]
    pub init: bool,

    /// Run initialization hooks and exit
    #[arg(long)]
    pub init_only: bool,

    /// Disable slash command parsing
    #[arg(long)]
    pub disable_slash_commands: bool,

    // ── NEW: Tool control ──────────────────────────────────────
    /// Restrict available tools (comma-separated)
    #[arg(long, value_name = "LIST", value_delimiter = ',')]
    pub tools: Option<Vec<String>>,

    /// Tools that execute without prompting (comma-separated patterns)
    #[arg(long, value_name = "PATTERNS", value_delimiter = ',')]
    pub allowed_tools: Option<Vec<String>>,

    /// Tools removed from model context entirely (comma-separated)
    #[arg(long, value_name = "PATTERNS", value_delimiter = ',')]
    pub disallowed_tools: Option<Vec<String>>,

    // ── Theme (CLI-315) ───────────────────────────────────────
    /// Select a named TUI color theme (e.g. intj, ocean, auto, daltonized)
    #[arg(long, value_name = "NAME")]
    pub theme: Option<String>,

    /// List available themes and exit
    #[arg(long)]
    pub list_themes: bool,

    // ── Output style (CLI-310) ─────────────────────────────────
    /// Select a named output style (e.g. Explanatory, Learning, Formal, Concise)
    #[arg(long, value_name = "NAME")]
    pub output_style: Option<String>,

    /// List available output styles and exit
    #[arg(long)]
    pub list_output_styles: bool,

    // ── Remote / headless ─────────────────────────────────────
    /// Run in headless mode (no TUI; JSON-lines on stdin/stdout for remote backend)
    #[arg(long)]
    pub headless: bool,

    /// Session ID for headless/remote mode (auto-generated if not provided)
    #[arg(long, value_name = "ID")]
    pub session_id: Option<String>,

    // ── NEW: Output ────────────────────────────────────────────
    /// Verbose logging with full turn-by-turn output
    #[arg(long)]
    pub verbose: bool,

    // ── NEW: Debugging ─────────────────────────────────────────
    /// Enable debug mode with optional category filter
    #[arg(long, value_name = "CATEGORIES")]
    pub debug: Option<Option<String>>,

    /// Write debug logs to specific file
    #[arg(long, value_name = "PATH")]
    pub debug_file: Option<PathBuf>,

    // ── Observability (TASK-TUI-803) ───────────────────────────
    /// Prometheus /metrics exporter port. When set (non-zero), spawns a
    /// loopback-only HTTP server at 127.0.0.1:<PORT>/metrics exposing the
    /// ChannelMetrics counters (backlog, throughput, p95 latency).
    ///
    /// Absent flag OR `--metrics-port 0` disables the exporter. Values below
    /// 1024 (privileged ports) are rejected at bind time — we deliberately
    /// do not pre-validate here so clap error messages match bind errors
    /// and avoid duplicating OS-specific rules.
    #[arg(long, value_name = "PORT")]
    pub metrics_port: Option<u16>,
}

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
        /// Pin to a specific version (e.g. "=1.0.0", "^2")
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

#[derive(Args, Debug, Clone)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub command: AuthSubcommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AuthSubcommand {
    Login {
        #[arg(long, value_enum, default_value = "anthropic")]
        provider: AuthProviderKind,
        #[arg(long, help = "Skip TOS warning prompt for this invocation only")]
        accept_tos: bool,
    },
    Status,
    Logout {
        #[arg(long, value_enum)]
        provider: Option<AuthProviderKind>,
    },
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthProviderKind {
    Anthropic,
    #[value(name = "openai-codex")]
    OpenaiCodex,
}

#[derive(Args, Debug, Clone)]
pub struct ChatArgs {
    /// Provider id (e.g. "anthropic", "openai-codex")
    #[arg(long, default_value = "anthropic")]
    pub provider: String,
    /// Model id override
    #[arg(long)]
    pub model: Option<String>,
    /// Disable streaming; print full response after completion
    #[arg(long)]
    pub no_stream: bool,
    /// Maximum output tokens
    #[arg(long, default_value_t = 1024)]
    pub max_tokens: u32,
    /// User prompt
    pub prompt: String,
}

#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProvidersAction {
    /// Show provider registry entries
    List,
    /// Show Archon surface support by provider/auth mode
    Capabilities,
    /// Diagnose local provider/auth configuration without live network calls
    Doctor,
}

#[derive(Subcommand, Debug)]
pub enum GametheoryAction {
    /// Run full pipeline: classify → route → specialists → report
    Run {
        /// The strategic situation to analyze
        situation: String,
        /// Tier 1 classification only (skip routing and specialists)
        #[arg(long)]
        classify_only: bool,
        /// Path to gametheory spec YAML (searches known locations if omitted)
        #[arg(long, value_name = "PATH")]
        spec_path: Option<String>,
        /// Bind the run to an ingested document/knowledge pack
        #[arg(long, value_name = "PACK")]
        kb: Option<String>,
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
    },
    /// List all persisted game-theory runs
    ListRuns,
    /// Show full details for a specific run
    Show {
        /// Run ID
        run_id: String,
    },
    /// Show status for one run, or status counts for all runs
    Status {
        /// Optional run ID
        run_id: Option<String>,
    },
    /// Inspect a run, specialist output, section, fingerprint, routing, or final report artifact
    Inspect {
        /// Artifact ID, e.g. gt-123, fingerprint:gt-123, specialist:gt-123:nash-equilibrium-finder
        artifact_id: String,
    },
    /// Inspect the Tier 1 fingerprint for a run
    InspectFingerprint {
        /// Run ID
        run_id: String,
    },
    /// Inspect the routing decision for a run
    InspectRouting {
        /// Run ID
        run_id: String,
    },
    /// Replay a run (re-evaluate routing from persisted fingerprint)
    Replay {
        /// Run ID
        run_id: String,
        /// Path to gametheory spec YAML (searches known locations if omitted)
        #[arg(long, value_name = "PATH")]
        spec_path: Option<String>,
        /// Re-run Tier 1 classification instead of preserving the stored fingerprint
        #[arg(long)]
        reclassify: bool,
        /// Re-run a single specialist using the stored Tier 1 fingerprint
        #[arg(long, value_name = "KEY")]
        rerun_specialist: Option<String>,
    },
    /// Resume an interrupted InProgress run from persisted checkpoints
    Resume {
        /// Run ID
        run_id: String,
        /// Path to gametheory spec YAML (searches known locations if omitted)
        #[arg(long, value_name = "PATH")]
        spec_path: Option<String>,
    },
    /// List curated game-theory agents
    ListAgents {
        /// Restrict output to a single tier
        #[arg(long, value_name = "N")]
        tier: Option<u8>,
    },
    /// List or ingest the known-fingerprint specimen library
    Specimens {
        /// Filter rows by axis=value, e.g. cooperation=cooperative
        #[arg(long, value_name = "AXIS=VALUE")]
        filter: Option<String>,
        /// Force re-ingest from the canonical markdown source
        #[arg(long)]
        ingest: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum CompletionAction {
    /// Run full completion-integrity check on a pipeline run
    Inspect {
        /// Run ID to inspect
        run_id: String,
        /// Task type for claim extraction (default: "pipeline-output")
        #[arg(long, default_value = "pipeline-output")]
        task_type: String,
    },
    /// List completion-sensitive claims for a run
    Claims {
        /// Run ID
        run_id: String,
    },
    /// List evidence records for a run
    Evidence {
        /// Run ID
        run_id: String,
    },
    /// List all false-completion incidents
    Incidents,
    /// Quick verify: run check and return pass/fail exit code
    Verify {
        /// Run ID to verify
        run_id: String,
        /// Task type for claim extraction
        #[arg(long, default_value = "pipeline-output")]
        task_type: String,
        /// Agent key responsible for the completion output
        #[arg(long, value_name = "KEY")]
        agent: Option<String>,
        /// Model responsible for the completion output
        #[arg(long, value_name = "NAME")]
        model: Option<String>,
        /// Workspace identifier for trust-score grouping
        #[arg(long, value_name = "ID")]
        workspace_id: Option<String>,
        /// Require at least one claim to exist (fail if none found)
        #[arg(long, default_value_t = false)]
        require_claims: bool,
    },
    /// Show persisted agent/model trust scores from completion verification history
    Trust {
        /// Filter to one agent key
        #[arg(long, value_name = "KEY")]
        agent: Option<String>,
        /// Filter to one model name
        #[arg(long, value_name = "NAME")]
        model: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum TeamAction {
    /// Run a named team on a goal
    Run {
        /// Team name defined in config
        #[arg(long)]
        team: String,
        /// Goal for the team to accomplish
        goal: String,
    },
    /// List configured teams
    List,
}

#[derive(Subcommand, Debug)]
pub enum PipelineAction {
    /// Run the coding pipeline on a task
    Code {
        /// Task description for the coding pipeline
        task: String,
        /// Display agent sequence and estimated cost without executing
        #[arg(long)]
        dry_run: bool,
    },
    /// Run the research pipeline on a topic
    Research {
        /// Research topic
        topic: String,
        /// Display agent sequence and estimated cost without executing
        #[arg(long)]
        dry_run: bool,
    },
    /// Show status of a pipeline session
    Status {
        /// Session ID
        session_id: String,
    },
    /// Resume an interrupted pipeline session
    Resume {
        /// Session ID to resume
        session_id: String,
    },
    /// List all pipeline sessions
    List,
    /// Abort a running pipeline session
    Abort {
        /// Session ID to abort
        session_id: String,
    },
    /// Run a declarative pipeline from a spec file
    #[command(name = "run")]
    Run {
        /// Path to pipeline spec file (YAML or JSON)
        file: std::path::PathBuf,
        /// Override format auto-detection (yaml or json)
        #[arg(long)]
        format: Option<String>,
        /// Return immediately after submission (don't poll for completion)
        #[arg(long)]
        detach: bool,
    },
    /// Cancel a running declarative pipeline
    #[command(name = "cancel")]
    Cancel {
        /// Pipeline run ID (UUID)
        id: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum RemoteAction {
    /// Connect to a remote agent via SSH
    Ssh {
        /// Target in user@host format (defaults to root@host if no @ present)
        target: String,
        /// One-shot command to run on the remote agent
        #[arg(long)]
        command: Option<String>,
        /// SSH port
        #[arg(long, default_value = "22")]
        port: u16,
        /// Path to SSH private key file
        #[arg(long)]
        key: Option<std::path::PathBuf>,
    },
    /// Connect to a remote agent via WebSocket
    Ws {
        /// WebSocket URL (e.g. ws://host:8420/ws)
        url: String,
        /// Bearer token for authentication
        #[arg(long)]
        token: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum KbAction {
    /// Ingest a file, URL, or directory into the knowledge base
    Ingest {
        /// Path or URL to ingest
        source: String,
        /// Domain tag for the ingested content
        #[arg(long)]
        domain: Option<String>,
    },
    /// List all nodes in the knowledge base
    List,
    /// Search for nodes matching a query string
    Search {
        /// Search query
        query: String,
        /// Maximum results
        #[arg(long, default_value = "10")]
        limit: usize,
        /// Retrieval mode: exact, semantic, or hybrid
        #[arg(long, default_value = "hybrid")]
        mode: String,
    },
    /// Extract claims, entities, relations, source quality and contradictions from doc chunks
    Process {
        /// Extract claims from document chunks
        #[arg(long)]
        claims: bool,
        /// Extract entities from document chunks
        #[arg(long)]
        entities: bool,
        /// Infer the knowledge graph relations
        #[arg(long, alias = "kg")]
        relations: bool,
        /// Scan claims for contradictions
        #[arg(long)]
        contradictions: bool,
    },
    /// List extracted claims
    Claims,
    /// List extracted entities
    Entities,
    /// List inferred relations
    Relations,
    /// List detected contradictions
    Contradictions,
    /// Show knowledge base statistics
    Stats,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DocsAction {
    /// Ingest a file or directory
    Ingest {
        /// Path to file or directory to ingest
        path: String,
    },
    /// List all ingested documents
    List,
    /// Show detailed information about a document
    Show {
        /// Document ID
        document_id: String,
    },
    /// Show document status summary
    Status,
    /// List chunks for a document
    Chunks {
        /// Document ID
        document_id: String,
    },
    /// Full inspection of a document (pages, chunks, OCR runs, provenance)
    Inspect {
        /// Document ID
        document_id: String,
    },
    /// Search for chunks relevant to a query
    Search {
        /// Search query
        query: String,
        /// Retrieval mode: exact, semantic, or hybrid
        #[arg(long, default_value = "hybrid")]
        mode: String,
        /// Show debug output (embedding details, distances, provenance)
        #[arg(long)]
        debug: bool,
    },
    /// Answer a question using document evidence
    Answer {
        /// Question to answer
        query: String,
    },
    /// Show provenance chain for a chunk or answer component
    Provenance {
        /// Chunk ID or answer component ID
        chunk_or_answer_id: String,
    },
    /// Index document chunks (embed and store vectors)
    Index {
        /// Re-index all chunks regardless of status
        #[arg(long)]
        all: bool,
    },
    /// Report embedding model and backend status
    ModelStatus,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ProvAction {
    /// Trace an artifact to its source lineage
    Trace {
        /// Artifact ID to trace
        artifact_id: String,
    },
    /// Export an artifact trace as W3C PROV JSON-LD
    Export {
        /// Artifact ID to export
        artifact_id: String,
    },
    /// Verify an artifact trace reaches source provenance
    Verify {
        /// Artifact ID to verify
        artifact_id: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum MeaningAction {
    /// Build meaning records from persisted learning signals
    Build {
        /// Source family to compile from
        #[arg(long, default_value = "learning-events")]
        from: String,
    },
    /// List derived samples
    Samples,
    /// List contrastive pairs
    Contrastive,
    /// List triplets
    Triplets,
    /// Export samples or triplets as JSONL
    Export {
        /// Dataset to export: samples or triplets
        #[arg(long, default_value = "samples")]
        kind: String,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum MemoryAction {
    /// Re-embed every memory in the graph using the currently-configured
    /// embedding model. Use after swapping models or recovering from a
    /// corrupted prior model. Existing vectors are overwritten in place.
    Reindex {
        /// Confirm a full re-embed (required — implicit guard against
        /// accidentally re-running an expensive operation).
        #[arg(long)]
        all: bool,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum ConstellationAction {
    /// Build a versioned centroid profile from positive meaning samples
    Build {
        /// Target profile: project, research-domain, or strategic-workflow
        #[arg(long)]
        target: String,
    },
    /// Score text or a file against the latest target centroid
    Score {
        /// Target profile to score against
        #[arg(long, default_value = "project")]
        target: String,
        /// File containing the answer/output to score
        #[arg(long)]
        answer: Option<PathBuf>,
        /// Inline text to score when --answer is not supplied
        #[arg(long)]
        text: Option<String>,
    },
    /// Detect whether text or a file has drifted from the target centroid
    Drift {
        /// Target profile to compare against
        #[arg(long, default_value = "project")]
        target: String,
        /// File containing the answer/output to inspect
        #[arg(long)]
        answer: Option<PathBuf>,
        /// Inline text to inspect when --answer is not supplied
        #[arg(long)]
        text: Option<String>,
        /// Minimum accepted similarity before drift is reported
        #[arg(long, default_value_t = 0.45)]
        threshold: f64,
    },
    /// List persisted constellation centroids
    List,
}

#[derive(Subcommand, Debug)]
pub enum PluginAction {
    /// List all discovered plugins with name, version, and status
    List,
    /// Show detailed information about a plugin
    Info {
        /// Plugin name
        name: String,
    },
}

/// Subcommands for `archon behaviour`
#[derive(Subcommand, Debug)]
pub enum BehaviourAction {
    /// List behaviour proposals (alias: proposals)
    #[command(alias = "proposals")]
    ListProposals,
    /// List learning events (optionally filtered by type)
    ListEvents {
        /// Filter by event type (e.g., FalseCompletionDetected, ManifestApplied)
        #[arg(short, long)]
        event_type: Option<String>,
    },
    /// Show details for a proposal, event, or manifest version
    Show {
        /// ID of the item to show (proposal_id, event_id, or version_id)
        id: String,
    },
    /// Auto-apply a pending proposal (without human review)
    Apply {
        /// Proposal ID to apply
        proposal_id: String,
    },
    /// Show version history for a manifest kind
    History {
        /// Manifest kind (RetrievalProfile, SourceQualityProfile, etc.)
        kind: String,
    },
    /// Generate proposals from recent learning events
    GenerateProposals,
    /// Show learning system status and statistics
    Status,
    /// Approve a pending proposal (human-in-the-loop)
    Approve {
        /// Proposal ID to approve
        proposal_id: String,
    },
    /// Deny a pending proposal
    Deny {
        /// Proposal ID to deny
        proposal_id: String,
    },
    /// Rollback a manifest to a previous version
    Rollback {
        /// Target version ID to rollback to
        version_id: String,
        /// Reason for rollback
        #[arg(short, long)]
        reason: Option<String>,
    },
}

impl Cli {
    /// Convert the clap-parsed Cli into a [`FlagInput`] for flag resolution.
    pub fn to_flag_input(&self) -> archon_core::cli_flags::FlagInput {
        archon_core::cli_flags::FlagInput {
            system_prompt: self.system_prompt.clone(),
            system_prompt_file: self.system_prompt_file.clone(),
            append_system_prompt: self.append_system_prompt.clone(),
            append_system_prompt_file: self.append_system_prompt_file.clone(),
            tools: self.tools.clone(),
            allowed_tools: self.allowed_tools.clone(),
            disallowed_tools: self.disallowed_tools.clone(),
            bare: self.bare,
            disable_slash_commands: self.disable_slash_commands,
            model: self.model.clone(),
            verbose: self.verbose,
            debug: self.debug.clone(),
            debug_file: self.debug_file.clone(),
            mcp_config: self.mcp_config.clone(),
            strict_mcp_config: self.strict_mcp_config,
            add_dir: self.add_dir.clone(),
            init: self.init,
            init_only: self.init_only,
            agent: self.agent.clone(),
        }
    }
}

#[cfg(test)]
mod metrics_port_parse_tests {
    //! AGS-OBS-903 Gate 4 coverage — pin `--metrics-port` clap parsing contract.
    //!
    //! Sherlock gate-3 flagged that without explicit parse tests the gate-walk
    //! on OBS-903 rested entirely on the smoke test, which skips CLI parsing.
    //! These pin the contract documented on the `metrics_port` field:
    //!   - absent flag         → `None`
    //!   - `--metrics-port 0`  → `Some(0)` (disables exporter at spawn site)
    //!   - `--metrics-port N`  → `Some(N)` for valid u16
    //!   - non-numeric value   → clap parse error
    //!   - value > u16::MAX    → clap parse error (overflow)
    use super::Cli;
    use clap::Parser;
    use clap::error::ErrorKind;
    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(args)
    }
    #[test]
    fn metrics_port_absent_is_none() {
        let cli = parse(&["archon"]).expect("no flags must parse");
        assert_eq!(cli.metrics_port, None);
    }
    #[test]
    fn metrics_port_zero_disables_but_parses() {
        let cli = parse(&["archon", "--metrics-port", "0"]).expect("zero must parse");
        assert_eq!(cli.metrics_port, Some(0));
    }
    #[test]
    fn metrics_port_valid_u16_parses() {
        let cli = parse(&["archon", "--metrics-port", "9090"]).expect("9090 must parse");
        assert_eq!(cli.metrics_port, Some(9090));
    }
    #[test]
    fn metrics_port_max_u16_parses() {
        let cli = parse(&["archon", "--metrics-port", "65535"]).expect("u16::MAX must parse");
        assert_eq!(cli.metrics_port, Some(65535));
    }
    #[test]
    fn metrics_port_non_numeric_rejected() {
        let err = parse(&["archon", "--metrics-port", "foo"]).expect_err("foo must fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }
    #[test]
    fn metrics_port_overflow_rejected() {
        let err = parse(&["archon", "--metrics-port", "70000"]).expect_err("70000 must fail");
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
    }
    #[test]
    fn metrics_port_negative_rejected() {
        // clap sees a leading `-` as a flag prefix, so `-1` surfaces as
        // `UnknownArgument` rather than `ValueValidation`. Either way the
        // contract we care about is: a negative value never becomes a bound
        // port. We pin both kinds so a future clap behaviour change forces us
        // to reread this note rather than silently accepting `-1`.
        let err = parse(&["archon", "--metrics-port", "-1"]).expect_err("negative must fail");
        assert!(
            matches!(
                err.kind(),
                ErrorKind::UnknownArgument | ErrorKind::ValueValidation
            ),
            "unexpected clap error kind for -1: {:?}",
            err.kind()
        );
    }
}

#[cfg(test)]
mod remote_url_parse_tests {
    //! TASK-TUI-625-FOLLOWUP Gate 4 coverage — pin `--remote-url` clap parsing
    //! contract. These tests guarantee that the long flag spelling stays
    //! `--remote-url` (hyphen, not underscore) and does NOT collide with the
    //! existing `Commands::Remote { action }` subcommand.
    use super::Cli;
    use clap::Parser;

    #[test]
    fn remote_url_parses_from_long_flag() {
        let cli =
            Cli::try_parse_from(["archon", "--remote-url", "https://archon.example/sess/xyz"])
                .expect("--remote-url <URL> must parse");
        assert_eq!(
            cli.remote_url.as_deref(),
            Some("https://archon.example/sess/xyz")
        );
    }

    #[test]
    fn remote_url_absent_when_not_supplied() {
        let cli = Cli::try_parse_from(["archon"]).expect("archon with no flags must parse");
        assert!(cli.remote_url.is_none());
    }
}

#[cfg(test)]
mod gametheory_prd_parse_tests {
    use super::{Cli, Commands, GametheoryAction};
    use clap::Parser;

    #[test]
    fn gametheory_prd_shorthand_parses_situation_and_kb() {
        let cli = Cli::try_parse_from([
            "archon",
            "gametheory",
            "Assess this plugin marketplace",
            "--kb",
            "policy-pack",
        ])
        .expect("PRD shorthand gametheory command must parse");

        match cli.command {
            Some(Commands::Gametheory {
                situation,
                kb,
                action,
                ..
            }) => {
                assert_eq!(situation.as_deref(), Some("Assess this plugin marketplace"));
                assert_eq!(kb.as_deref(), Some("policy-pack"));
                assert!(action.is_none());
            }
            other => panic!("expected gametheory command, got {other:?}"),
        }
    }

    #[test]
    fn gametheory_prd_classify_only_shorthand_parses() {
        let cli = Cli::try_parse_from([
            "archon",
            "gametheory",
            "--classify-only",
            "Assess a bargaining situation",
        ])
        .expect("PRD classify-only shorthand must parse");

        match cli.command {
            Some(Commands::Gametheory {
                situation,
                classify_only,
                action,
                ..
            }) => {
                assert_eq!(situation.as_deref(), Some("Assess a bargaining situation"));
                assert!(classify_only);
                assert!(action.is_none());
            }
            other => panic!("expected gametheory command, got {other:?}"),
        }
    }

    #[test]
    fn gametheory_existing_run_subcommand_keeps_kb_flag() {
        let cli = Cli::try_parse_from([
            "archon",
            "gametheory",
            "run",
            "Assess a deterrence game",
            "--kb",
            "policy-pack",
        ])
        .expect("existing run subcommand must still parse");

        match cli.command {
            Some(Commands::Gametheory {
                action: Some(GametheoryAction::Run { situation, kb, .. }),
                ..
            }) => {
                assert_eq!(situation, "Assess a deterrence game");
                assert_eq!(kb.as_deref(), Some("policy-pack"));
            }
            other => panic!("expected gametheory run action, got {other:?}"),
        }
    }
}
