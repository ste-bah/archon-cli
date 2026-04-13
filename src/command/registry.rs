//! Slash command registry.
//!
//! TASK-AGS-622: typed command table. Replaces the implicit mapping
//! embedded in `handle_slash_command`'s monolithic `match` block with
//! an explicit `HashMap<&'static str, Arc<dyn CommandHandler>>`.
//!
//! This module establishes the structural shape only. Handler bodies
//! are intentional no-op stubs returning `Ok(())`; TASK-AGS-624 (or a
//! Phase 8 follow-up) migrates the real per-command logic out of
//! `main.rs::handle_slash_command`. Keeping the shape here lets Phase 8
//! register new commands by adding entries instead of editing `main.rs`.
//!
//! Declared `pub(crate)` from `src/command/mod.rs` so visibility is
//! scoped to the bin crate (the `archon-cli` library target does not
//! see this module).

use std::collections::HashMap;
use std::sync::Arc;

use archon_tui::app::TuiEvent;

/// Execution context threaded through every command handler.
///
/// Kept deliberately minimal for TASK-AGS-622: the registry's job is
/// shape, not plumbing. TASK-AGS-623 (dispatcher) grows this struct to
/// carry the real `SlashCommandContext` fields (fast mode, effort,
/// memory, config, etc.) once handlers are migrated off `main.rs`.
pub(crate) struct CommandContext {
    /// TUI event sink for text deltas, errors, and state change
    /// notifications.
    pub(crate) tui_tx: tokio::sync::mpsc::Sender<TuiEvent>,
}

/// Trait every registered slash command handler implements.
///
/// `execute` runs the handler against the supplied context and
/// positional argument list. `description` is a one-line human label
/// used by `/help`, the command picker, and future introspection.
pub(crate) trait CommandHandler: Send + Sync {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()>;
    fn description(&self) -> &str;
}

/// Typed command table.
///
/// Owns `Arc<dyn CommandHandler>` so the dispatcher can clone handlers
/// out of the map cheaply and invoke them without holding a borrow on
/// the registry. Insertion order is irrelevant; lookup is by name.
pub(crate) struct Registry {
    commands: HashMap<&'static str, Arc<dyn CommandHandler>>,
}

impl Registry {
    /// Look up a registered handler by command name (without the
    /// leading `/`). Returns a cloned `Arc`, or `None` if no handler
    /// is registered under that name.
    pub(crate) fn get(&self, name: &str) -> Option<Arc<dyn CommandHandler>> {
        self.commands.get(name).cloned()
    }

    /// Number of registered commands. Primarily for tests / `/help`.
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.commands.len()
    }
}

// ---------------------------------------------------------------------------
// Handler placeholders
// ---------------------------------------------------------------------------
//
// Every existing slash command gets a zero-sized handler struct with a
// stub `execute` body. TASK-AGS-624 will migrate the real handler logic
// out of `main.rs::handle_slash_command` into these `execute` bodies.
// The macro below keeps each declaration to a single line so the file
// stays well under the 500-line budget.

macro_rules! declare_handler {
    ($struct_name:ident, $description:literal) => {
        struct $struct_name;
        impl CommandHandler for $struct_name {
            fn execute(
                &self,
                _ctx: &mut CommandContext,
                _args: &[String],
            ) -> anyhow::Result<()> {
                // TASK-AGS-624 will migrate the real handler body here
                // from main.rs::handle_slash_command.
                Ok(())
            }
            fn description(&self) -> &str {
                $description
            }
        }
    };
}

declare_handler!(FastHandler, "Toggle fast mode (lower quality, faster responses)");
declare_handler!(CompactHandler, "Compact the current conversation history");
declare_handler!(ClearHandler, "Clear the current conversation");
declare_handler!(ExportHandler, "Export the current session to a file");
declare_handler!(ThinkingHandler, "Toggle extended thinking display on/off");
declare_handler!(EffortHandler, "Show or set reasoning effort (high|medium|low)");
declare_handler!(GardenHandler, "Run memory garden consolidation or show stats");
declare_handler!(ModelHandler, "Show or switch the active model");
declare_handler!(CopyHandler, "Copy the last assistant message to the clipboard");
declare_handler!(ContextHandler, "Show current context window usage");
declare_handler!(StatusHandler, "Show session status (model, effort, token use)");
declare_handler!(CostHandler, "Show session token cost breakdown");
declare_handler!(PermissionsHandler, "Show or update tool permissions");
declare_handler!(ConfigHandler, "Show or update Archon configuration");
declare_handler!(MemoryHandler, "Inspect or manage long-term memory");
declare_handler!(DoctorHandler, "Run environment health checks");
declare_handler!(BugHandler, "Report a bug with current session context");
declare_handler!(DiffHandler, "Show a diff of recent file modifications");
declare_handler!(DenialsHandler, "List tool-use denials recorded this session");
declare_handler!(LoginHandler, "Authenticate against the configured backend");
declare_handler!(VimHandler, "Toggle vim-style modal input");
declare_handler!(UsageHandler, "Show aggregate API usage for the session");
declare_handler!(TasksHandler, "List or manage project tasks");
declare_handler!(ReleaseNotesHandler, "Show release notes for the current build");
declare_handler!(ReloadHandler, "Reload configuration from disk");
declare_handler!(LogoutHandler, "Clear stored credentials");
declare_handler!(HelpHandler, "Show help for commands and shortcuts");
declare_handler!(RenameHandler, "Rename the current session");
declare_handler!(ResumeHandler, "Resume a previous session by id");
declare_handler!(McpHandler, "Show MCP server status");
declare_handler!(ForkHandler, "Fork the current session into a new branch");
declare_handler!(CheckpointHandler, "Create or restore a session checkpoint");
declare_handler!(AddDirHandler, "Add a directory to the working context");
declare_handler!(ColorHandler, "Show or change the UI color scheme");
declare_handler!(ThemeHandler, "Show or change the UI theme");
declare_handler!(RecallHandler, "Recall memories matching a query");
declare_handler!(RulesHandler, "List, edit, or remove behavioral rules");

/// Build the default command registry containing every slash command
/// currently dispatched from `main.rs::handle_slash_command`.
///
/// Each command name maps to a `pub(crate)` zero-sized handler struct
/// whose `execute` body is a no-op stub. Migrating the real bodies
/// out of `handle_slash_command` is scoped to TASK-AGS-624 / Phase 8.
pub(crate) fn default_registry() -> Registry {
    let mut commands: HashMap<&'static str, Arc<dyn CommandHandler>> = HashMap::new();
    commands.insert("fast", Arc::new(FastHandler));
    commands.insert("compact", Arc::new(CompactHandler));
    commands.insert("clear", Arc::new(ClearHandler));
    commands.insert("export", Arc::new(ExportHandler));
    commands.insert("thinking", Arc::new(ThinkingHandler));
    commands.insert("effort", Arc::new(EffortHandler));
    commands.insert("garden", Arc::new(GardenHandler));
    commands.insert("model", Arc::new(ModelHandler));
    commands.insert("copy", Arc::new(CopyHandler));
    commands.insert("context", Arc::new(ContextHandler));
    commands.insert("status", Arc::new(StatusHandler));
    commands.insert("cost", Arc::new(CostHandler));
    commands.insert("permissions", Arc::new(PermissionsHandler));
    commands.insert("config", Arc::new(ConfigHandler));
    commands.insert("memory", Arc::new(MemoryHandler));
    commands.insert("doctor", Arc::new(DoctorHandler));
    commands.insert("bug", Arc::new(BugHandler));
    commands.insert("diff", Arc::new(DiffHandler));
    commands.insert("denials", Arc::new(DenialsHandler));
    commands.insert("login", Arc::new(LoginHandler));
    commands.insert("vim", Arc::new(VimHandler));
    commands.insert("usage", Arc::new(UsageHandler));
    commands.insert("tasks", Arc::new(TasksHandler));
    commands.insert("release-notes", Arc::new(ReleaseNotesHandler));
    commands.insert("reload", Arc::new(ReloadHandler));
    commands.insert("logout", Arc::new(LogoutHandler));
    commands.insert("help", Arc::new(HelpHandler));
    commands.insert("rename", Arc::new(RenameHandler));
    commands.insert("resume", Arc::new(ResumeHandler));
    commands.insert("mcp", Arc::new(McpHandler));
    commands.insert("fork", Arc::new(ForkHandler));
    commands.insert("checkpoint", Arc::new(CheckpointHandler));
    commands.insert("add-dir", Arc::new(AddDirHandler));
    commands.insert("color", Arc::new(ColorHandler));
    commands.insert("theme", Arc::new(ThemeHandler));
    commands.insert("recall", Arc::new(RecallHandler));
    commands.insert("rules", Arc::new(RulesHandler));
    Registry { commands }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Count of distinct command names extracted from the 37 match
    /// arms in `main.rs::handle_slash_command` as of TASK-AGS-622.
    /// Two of those arms (`/compact | /clear` and the `/thinking`
    /// family) contribute separately-named commands, so the final
    /// count is 37 unique names.
    const EXPECTED_COMMAND_COUNT: usize = 37;

    #[test]
    fn default_registry_contains_all_commands() {
        let registry = default_registry();
        assert_eq!(
            registry.len(),
            EXPECTED_COMMAND_COUNT,
            "default_registry must register every pre-TASK-AGS-622 slash command"
        );
    }

    #[test]
    fn default_registry_includes_fast() {
        assert!(default_registry().get("fast").is_some());
    }

    #[test]
    fn default_registry_includes_help() {
        assert!(default_registry().get("help").is_some());
    }

    #[test]
    fn default_registry_includes_config() {
        assert!(default_registry().get("config").is_some());
    }

    #[test]
    fn default_registry_includes_rules() {
        assert!(default_registry().get("rules").is_some());
    }

    #[test]
    fn default_registry_includes_thinking() {
        assert!(default_registry().get("thinking").is_some());
    }

    #[test]
    fn default_registry_includes_compact_and_clear_separately() {
        let registry = default_registry();
        assert!(registry.get("compact").is_some());
        assert!(registry.get("clear").is_some());
    }

    #[test]
    fn unknown_command_returns_none() {
        assert!(default_registry().get("nonexistent").is_none());
    }

    #[test]
    fn handler_description_is_non_empty() {
        let registry = default_registry();
        let handler = registry.get("fast").expect("fast handler registered");
        assert!(!handler.description().is_empty());
    }

    #[test]
    fn registry_lookup_returns_arc() {
        let registry = default_registry();
        let first = registry.get("fast");
        let second = registry.get("fast");
        assert!(first.is_some());
        assert!(second.is_some());
    }
}
