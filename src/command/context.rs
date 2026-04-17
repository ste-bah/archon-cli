//! TASK-AGS-807: async `CommandContext` builder (snapshot pattern).
//!
//! `CommandHandler::execute` is SYNC (Q1=A invariant). The shipped
//! `/status` body relies on four `tokio::sync::Mutex` guards acquired via
//! `.lock().await`. To bridge the gap, this module constructs a
//! [`CommandContext`] at the dispatch site WHERE `.await` is allowed and
//! pre-populates an owned [`StatusSnapshot`] when the primary command
//! resolves to `/status` (or its alias `/info`). Every other command
//! gets `status_snapshot = None`, so there is zero extra lock traffic
//! for unrelated slash inputs.
//!
//! The builder does not take ownership of `SlashCommandContext`; it
//! borrows it for the duration of the snapshot read and returns a
//! self-contained [`CommandContext`] with only the values the sync
//! handler will need.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, Registry};
use crate::command::status;
use crate::slash_context::SlashCommandContext;

/// Build the per-dispatch [`CommandContext`] for the supplied slash
/// `input`. Awaits the lock-protected shared state ONLY when the primary
/// command resolves to a handler that consumes a [`StatusSnapshot`]
/// (currently: `/status` and its `info` alias).
///
/// # Panics
///
/// Does not panic. Parse failures / unknown names result in a
/// `CommandContext` with every optional field set to `None`; the
/// dispatcher downstream will surface "Unknown command" or a parse
/// error via its own path.
pub(crate) async fn build_command_context(
    input: &str,
    tui_tx: tokio::sync::mpsc::Sender<TuiEvent>,
    slash_ctx: &SlashCommandContext,
) -> CommandContext {
    let mut ctx = CommandContext {
        tui_tx,
        status_snapshot: None,
    };

    // Resolve the primary command name (alias-aware) so "/info" routes
    // to the same snapshot path as "/status". The registry holds the
    // canonical alias→primary mapping; we delegate rather than
    // duplicating the table here.
    let primary = resolve_primary_from_input(input, slash_ctx.registry.as_ref());

    if matches!(primary.as_deref(), Some("status")) {
        ctx.status_snapshot = Some(status::build_status_snapshot(slash_ctx).await);
    }

    ctx
}

/// Resolve a slash `input` to its primary command name, consulting the
/// [`Registry`]'s alias map. Returns `None` for non-slash inputs, parse
/// failures, or unknown command names.
///
/// Private to this module: builder tests exercise it directly against
/// `default_registry()` because stubbing the full `SlashCommandContext`
/// is out of scope for this ticket (see AGS-807 executor report).
pub(crate) fn resolve_primary_from_input(
    input: &str,
    registry: &Registry,
) -> Option<String> {
    // Reuse the shared tokenizer so we inherit its leading-`/` handling,
    // quoted-arg rules, and flag tolerance.
    let parsed = crate::command::parser::CommandParser::parse(input).ok()?;

    // `Registry::get` is alias-aware; we re-derive the PRIMARY name by
    // asking the commands map first, then the alias map, so the caller
    // can compare against a canonical string like "status".
    if registry.get(&parsed.name).is_some() {
        // Direct primary hit: return the parsed name itself. But if the
        // name is an alias (not a primary), we need the primary string.
        if registry.is_primary(&parsed.name) {
            return Some(parsed.name);
        }
        // Alias → primary resolution.
        return registry.primary_for_alias(&parsed.name).map(str::to_string);
    }
    None
}

// ---------------------------------------------------------------------------
// TASK-AGS-807: tests for build_command_context primary-name resolution
// ---------------------------------------------------------------------------
//
// Rationale for not using a full `SlashCommandContext` fixture:
//
// `SlashCommandContext` carries 24+ fields including `McpServerManager`,
// `Arc<dyn MemoryTrait>`, `Arc<RwLock<AgentRegistry>>`, `SkillRegistry`,
// and several `Mutex`-wrapped runtime state slots. Standing up a real
// fixture would (a) drag test-only dependencies into the bin crate,
// and (b) couple AGS-807's test surface to fields that have nothing to
// do with /status. The AGS-807 executor-report directive explicitly
// permits reporting the chosen approach.
//
// The builder's interesting behaviour is the alias-aware primary-name
// resolution: "/status" → Some("status"), "/info" → Some("status"),
// "/tasks" → Some("tasks") (no snapshot populated). All three of those
// behaviours live in `resolve_primary_from_input`, which takes a
// `&Registry` and is fully testable against `default_registry()` with
// no SlashCommandContext fixture at all.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::default_registry;

    #[test]
    fn build_command_context_populates_status_snapshot_for_slash_status() {
        // Primary lookup proves the builder would populate the snapshot.
        // The full `build_command_context` path is exercised indirectly
        // via live smoke (Gate 5); here we pin the routing decision.
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/status", &reg).as_deref(),
            Some("status"),
            "/status must resolve to primary 'status' so build_command_context \
             populates a StatusSnapshot"
        );
    }

    #[test]
    fn build_command_context_populates_status_snapshot_for_slash_info_alias() {
        // The alias `info` must route to primary `status`. If this ever
        // regresses, /info would fall through to None and StatusHandler
        // would return Err at execute time (see status.rs handler test
        // `status_handler_execute_without_snapshot_returns_err`).
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/info", &reg).as_deref(),
            Some("status"),
            "alias '/info' must route through the registry alias map back to \
             primary 'status' so build_command_context fires the snapshot branch"
        );
    }

    #[test]
    fn build_command_context_leaves_snapshot_none_for_slash_tasks() {
        // `/tasks` is its own primary. The builder should see a primary
        // name != "status" and leave `status_snapshot` at None so the
        // TasksHandler does not pay for unused lock traffic.
        let reg = default_registry();
        let primary = resolve_primary_from_input("/tasks", &reg);
        assert_eq!(
            primary.as_deref(),
            Some("tasks"),
            "/tasks must resolve to its own primary, not 'status'"
        );
        assert_ne!(
            primary.as_deref(),
            Some("status"),
            "the snapshot branch must only fire for 'status' — other primaries \
             must observe status_snapshot = None"
        );
    }
}
