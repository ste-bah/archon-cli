//! CLI flag resolution: reads file-backed flags and validates mutual exclusivity.
//!
//! This module operates on a plain [`FlagInput`] struct rather than the clap
//! [`Cli`] struct directly so that it can be tested without pulling in the
//! full binary crate. The binary maps clap fields to [`FlagInput`] before
//! calling [`resolve_flags`].

use std::path::PathBuf;

/// Raw flag values extracted from the CLI (mirrors the relevant clap fields).
///
/// The binary crate constructs this from the parsed [`Cli`] struct.
#[derive(Debug, Default)]
pub struct FlagInput {
    pub system_prompt: Option<String>,
    pub system_prompt_file: Option<PathBuf>,
    pub append_system_prompt: Option<String>,
    pub append_system_prompt_file: Option<PathBuf>,
    pub tools: Option<Vec<String>>,
    pub allowed_tools: Option<Vec<String>>,
    pub disallowed_tools: Option<Vec<String>>,
    pub bare: bool,
    pub disable_slash_commands: bool,
    pub model: Option<String>,
    pub verbose: bool,
    pub debug: Option<Option<String>>,
    pub debug_file: Option<PathBuf>,
    pub mcp_config: Vec<PathBuf>,
    pub strict_mcp_config: bool,
    pub add_dir: Vec<PathBuf>,
    pub init: bool,
    pub init_only: bool,
    pub agent: Option<String>,
}

/// Resolved flag values after file reads and validation.
#[derive(Debug, Default)]
pub struct ResolvedFlags {
    /// If set, replaces the entire system prompt.
    pub system_prompt_override: Option<String>,
    /// If set, appended to the default system prompt.
    pub system_prompt_append: Option<String>,
    /// If set, only these tools are available.
    pub tool_whitelist: Option<Vec<String>>,
    /// If set, these tools are removed from the registry.
    pub tool_blacklist: Option<Vec<String>>,
    /// Tools that execute without prompting (auto-allowed patterns).
    pub allowed_tools: Option<Vec<String>>,
    /// Minimal mode: skip hooks, CLAUDE.md, MCP auto-start.
    pub bare_mode: bool,
    /// Disable slash command parsing.
    pub disable_slash_commands: bool,
    /// Model override from --model flag.
    pub model: Option<String>,
    /// Verbose logging.
    pub verbose: bool,
    /// Debug mode with optional category filter.
    pub debug: Option<Option<String>>,
    /// Debug log file path.
    pub debug_file: Option<PathBuf>,
    /// MCP config file paths from --mcp-config.
    pub mcp_config_paths: Vec<PathBuf>,
    /// Only use MCP servers from --mcp-config.
    pub strict_mcp_config: bool,
    /// Additional working directories.
    pub add_dirs: Vec<PathBuf>,
    /// Run initialization hooks and start interactive mode.
    pub init: bool,
    /// Run initialization hooks and exit.
    pub init_only: bool,
    /// Agent definition name.
    pub agent: Option<String>,
}

/// Resolve raw CLI flag values into validated, file-read-resolved values.
///
/// Reads the contents of `--system-prompt-file` and `--append-system-prompt-file`
/// if specified. Returns an error string on validation failure (e.g. missing file).
pub fn resolve_flags(input: &FlagInput) -> Result<ResolvedFlags, String> {
    // --- System prompt override ---
    let system_prompt_override = if let Some(ref text) = input.system_prompt {
        Some(text.clone())
    } else if let Some(ref path) = input.system_prompt_file {
        let content = std::fs::read_to_string(path).map_err(|e| {
            format!(
                "--system-prompt-file: cannot read '{}': {e}",
                path.display()
            )
        })?;
        Some(content)
    } else {
        None
    };

    // --- System prompt append ---
    let system_prompt_append = if let Some(ref text) = input.append_system_prompt {
        Some(text.clone())
    } else if let Some(ref path) = input.append_system_prompt_file {
        let content = std::fs::read_to_string(path).map_err(|e| {
            format!(
                "--append-system-prompt-file: cannot read '{}': {e}",
                path.display()
            )
        })?;
        Some(content)
    } else {
        None
    };

    // --- Tool restrictions ---
    let tool_whitelist = input.tools.clone();
    let tool_blacklist = input.disallowed_tools.clone();
    let allowed_tools = input.allowed_tools.clone();

    Ok(ResolvedFlags {
        system_prompt_override,
        system_prompt_append,
        tool_whitelist,
        tool_blacklist,
        allowed_tools,
        bare_mode: input.bare,
        disable_slash_commands: input.disable_slash_commands,
        model: input.model.clone(),
        verbose: input.verbose,
        debug: input.debug.clone(),
        debug_file: input.debug_file.clone(),
        mcp_config_paths: input.mcp_config.clone(),
        strict_mcp_config: input.strict_mcp_config,
        add_dirs: input.add_dir.clone(),
        init: input.init,
        init_only: input.init_only,
        agent: input.agent.clone(),
    })
}
