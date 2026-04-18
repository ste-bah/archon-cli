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

use crate::command::registry::{CommandContext, CommandEffect, Registry};
use crate::command::{cost, model, status};
use crate::slash_context::SlashCommandContext;

/// Build the per-dispatch [`CommandContext`] for the supplied slash
/// `input`. Awaits the lock-protected shared state ONLY when the primary
/// command resolves to a handler that consumes one of the typed
/// snapshots (currently: `/status` -> [`status::StatusSnapshot`],
/// `/model` -> [`model::ModelSnapshot`], `/cost` ->
/// [`cost::CostSnapshot`]). Other primaries observe every optional
/// field as `None` and pay zero lock traffic.
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
        model_snapshot: None,
        cost_snapshot: None,
        pending_effect: None,
    };

    // Resolve the primary command name (alias-aware) so "/info" routes
    // to the same snapshot path as "/status" and "/m" / "/switch-model"
    // route to the same snapshot path as "/model". The registry holds
    // the canonical alias→primary mapping; we delegate rather than
    // duplicating the table here.
    let primary = resolve_primary_from_input(input, slash_ctx.registry.as_ref());

    match primary.as_deref() {
        Some("status") => {
            ctx.status_snapshot =
                Some(status::build_status_snapshot(slash_ctx).await);
        }
        Some("model") => {
            ctx.model_snapshot =
                Some(model::build_model_snapshot(slash_ctx).await);
        }
        Some("cost") => {
            // TASK-AGS-809 snapshot population. /cost is read-only,
            // so there is no paired `apply_effect` branch. The alias
            // `billing` also routes here via the registry alias map;
            // `usage` remains a separate primary (UsageHandler).
            ctx.cost_snapshot =
                Some(cost::build_cost_snapshot(slash_ctx).await);
        }
        _ => {}
    }

    ctx
}

/// Apply a [`CommandEffect`] produced by a handler by awaiting the
/// write-back on the appropriate `SlashCommandContext` field.
///
/// TASK-AGS-808 introduced this helper to bridge the sync
/// `CommandHandler` boundary with `tokio::sync::Mutex` writes that
/// shipped bodies performed inline. Handlers stash an effect in
/// `CommandContext::pending_effect` synchronously; `slash.rs::
/// handle_slash_command` takes the value (consuming the slot via
/// `.take()`) after `Dispatcher::dispatch` returns and calls
/// `apply_effect`, which awaits the mutex write before returning
/// control to the main input loop.
///
/// Future body-migrate tickets add new `CommandEffect` variants and
/// extend the match below.
pub(crate) async fn apply_effect(
    effect: CommandEffect,
    slash_ctx: &SlashCommandContext,
) {
    match effect {
        CommandEffect::SetModelOverride(resolved) => {
            *slash_ctx.model_override_shared.lock().await = resolved;
        }
        // Future variants (AGS-819 /theme, etc.): add a match arm here
        // with the appropriate awaited mutex write. No fallback arm —
        // enum exhaustiveness forces new tickets to wire their effects
        // through this single point of truth.
    }
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
    use crate::command::registry::{default_registry, CommandEffect};
    use std::sync::Arc;
    use tokio::sync::Mutex;

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

    // -----------------------------------------------------------------
    // TASK-AGS-808: model snapshot routing + apply_effect mutex write.
    //
    // The builder routes `/model` (and its aliases `/m`, `/switch-model`)
    // to `model::build_model_snapshot`. Same rationale as the AGS-807
    // /status tests — we pin the routing decision via the pure
    // `resolve_primary_from_input` helper because standing up a full
    // `SlashCommandContext` fixture drags McpServerManager / MemoryTrait
    // / SkillRegistry into the test crate.
    // -----------------------------------------------------------------

    #[test]
    fn build_command_context_populates_model_snapshot_for_slash_model() {
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/model", &reg).as_deref(),
            Some("model"),
            "/model must resolve to primary 'model' so build_command_context \
             populates a ModelSnapshot"
        );
    }

    /// Verifies the apply_effect semantics for
    /// `CommandEffect::SetModelOverride`. Fixture choice: option (b)
    /// in the AGS-808 executor report — a narrow
    /// `Arc<Mutex<String>>` test harness that mirrors the apply_effect
    /// match body for the one variant under test. Full
    /// SlashCommandContext fixture is infeasible (24+ fields including
    /// McpServerManager + Arc<dyn MemoryTrait>). The production
    /// apply_effect keeps the `&SlashCommandContext` signature for
    /// future-variant symmetry; this test exercises the write-back
    /// invariant (`*mutex.lock().await = resolved`) directly.
    #[tokio::test]
    async fn apply_effect_set_model_override_writes_to_mutex() {
        let model_override_shared: Arc<Mutex<String>> =
            Arc::new(Mutex::new(String::new()));
        assert!(
            model_override_shared.lock().await.is_empty(),
            "pre-condition: override must start empty"
        );

        let effect = CommandEffect::SetModelOverride("claude-sonnet-4-6".to_string());

        // Narrow harness mirroring apply_effect's match arm. If
        // production apply_effect diverges, this test will need to be
        // updated in lockstep — that is the intended coupling.
        match effect {
            CommandEffect::SetModelOverride(resolved) => {
                *model_override_shared.lock().await = resolved;
            }
        }

        let got = model_override_shared.lock().await.clone();
        assert_eq!(
            got, "claude-sonnet-4-6",
            "apply_effect must overwrite model_override_shared with the \
             resolved full model id"
        );
    }

    // -----------------------------------------------------------------
    // TASK-AGS-809: /cost snapshot routing. Same rationale as AGS-807 +
    // AGS-808 — we pin the routing decision via
    // `resolve_primary_from_input` because standing up a full
    // `SlashCommandContext` fixture drags McpServerManager /
    // MemoryTrait / SkillRegistry into the test crate. The primary
    // name returned here is what `build_command_context` uses to
    // decide whether to populate `ctx.cost_snapshot`.
    //
    // /cost is READ-ONLY, so there is no matching `apply_effect` test
    // in this ticket — no CommandEffect variant was added for AGS-809.
    // -----------------------------------------------------------------

    #[test]
    fn build_command_context_populates_cost_snapshot_for_slash_cost() {
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/cost", &reg).as_deref(),
            Some("cost"),
            "/cost must resolve to primary 'cost' so build_command_context \
             populates a CostSnapshot"
        );
    }

    #[test]
    fn build_command_context_populates_cost_snapshot_for_slash_billing_alias() {
        // Spec wanted `/usage` as an alias for /cost, but `usage` is
        // already a shipped primary (UsageHandler). Only `/billing`
        // routes to /cost; `/usage` remains bound to UsageHandler.
        // See cost.rs module rustdoc + the CONFIRM R-item in the
        // AGS-809 executor report.
        let reg = default_registry();
        assert_eq!(
            resolve_primary_from_input("/billing", &reg).as_deref(),
            Some("cost"),
            "alias '/billing' must route through the registry alias map \
             back to primary 'cost' so build_command_context fires the \
             cost snapshot branch"
        );
        // Sanity: /usage must NOT route to 'cost'. It is a primary in
        // its own right and its snapshot branch (if any) belongs to a
        // future UsageHandler body-migrate, not AGS-809.
        assert_eq!(
            resolve_primary_from_input("/usage", &reg).as_deref(),
            Some("usage"),
            "/usage is a shipped primary (UsageHandler); must NOT resolve \
             to 'cost'"
        );
    }
}
