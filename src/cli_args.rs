//! CLI argument definitions for the `archon` binary.
//!
//! Extracted from `main.rs` so the Cli struct can grow without bloating the
//! main module. All clap derive definitions live here.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Archon CLI -- Rust-native AI agent runtime
#[derive(Parser, Debug)]
#[command(name = "archon")]
#[command(version = "0.1.0")]
#[command(about = "Archon CLI -- Rust-native AI agent runtime", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    // ── Existing flags ─────────────────────────────────────────

    /// Resume a previous session (list recent or specify ID)
    #[arg(long)]
    pub resume: Option<Option<String>>,

    /// Enable fast mode (reduced latency, lower quality)
    #[arg(long)]
    pub fast: bool,

    /// Set reasoning effort level (high, medium, low)
    #[arg(long, value_name = "LEVEL")]
    pub effort: Option<String>,

    /// Enable identity spoofing (mimic Claude Code headers)
    #[arg(long)]
    pub identity_spoof: bool,

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

    /// Minimal mode: skip hooks, CLAUDE.md, MCP auto-start
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
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Authenticate with Anthropic via OAuth PKCE flow
    Login,
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
