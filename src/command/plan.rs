//! TASK-TUI-626 /plan slash-command handler (Gate 1 skeleton).
//!
//! `/plan` enables Plan Mode — a permission mode where the user approves
//! each tool call individually.
//!
//! # Architecture
//!
//! Uses the existing SNAPSHOT+EFFECT plumbing from `/permissions`
//! (`src/command/permissions.rs`). Handler body is small: emit
//! confirmation TextDelta + stash `CommandEffect::SetPermissionMode("plan")`.
//! The dispatcher's `apply_effect` post-handler does the async write
//! to `slash_ctx.permission_mode.lock().await` AND emits
//! `TuiEvent::PermissionModeChanged("plan")`.
//!
//! # Scope boundary
//!
//! Spec's `/plan [open]` sub-argument (opens `.archon/plan.md` in
//! `$EDITOR`), plan-file content display, and plan-file I/O helpers
//! are scoped to P0-B.3 (issue #174) — deferred. This ticket lands
//! ONLY the mode-toggle path.
//!
//! # Reconciliation with TASK-TUI-626.md spec
//!
//! Spec references `crates/archon-tui/src/slash/plan.rs` +
//! `SlashCommand` + `SlashOutcome::Message`. Actual: bin-crate
//! `src/command/plan.rs` + `CommandHandler` (re-exported as
//! `SlashCommand` at `src/command/mod.rs:86`). Mode write goes via
//! `ctx.pending_effect = Some(CommandEffect::SetPermissionMode("plan"))`,
//! not direct enum assignment — matches the `/permissions` precedent
//! for sync `CommandHandler::execute` + async mutex write.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};

/// `/plan` handler — enables Plan Mode via the SNAPSHOT+EFFECT pattern.
pub(crate) struct PlanHandler;

impl CommandHandler for PlanHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // Emit confirmation TextDelta BEFORE stashing the effect
        // (B10/B11 emission-order precedent — see permissions.rs R6).
        ctx.emit(TuiEvent::TextDelta(
            "\nPlan mode enabled. You will be asked to approve each tool call.\n"
                .to_string(),
        ));

        // Stash the shared-mutex write. apply_effect at the dispatch
        // site performs:
        //   *slash_ctx.permission_mode.lock().await = PermissionMode::Plan
        // AND emits TuiEvent::PermissionModeChanged("plan") AFTER the
        // write lands.
        ctx.pending_effect = Some(CommandEffect::SetPermissionMode("plan".to_string()));
        Ok(())
    }

    fn description(&self) -> &str {
        "Enable Plan Mode (approve each tool call individually)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    #[test]
    fn plan_emits_confirmation_textdelta() {
        let (mut ctx, mut rx) = make_bug_ctx();
        PlanHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.to_lowercase().contains("plan mode"),
                    "TextDelta must mention 'Plan mode'; got: {}",
                    s
                );
                assert!(
                    s.starts_with('\n') && s.ends_with('\n'),
                    "TextDelta must carry leading+trailing \\n wrap; got: {:?}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn plan_stashes_set_permission_mode_effect() {
        let (mut ctx, _rx) = make_bug_ctx();
        PlanHandler.execute(&mut ctx, &[]).unwrap();
        match ctx.pending_effect {
            Some(CommandEffect::SetPermissionMode(ref mode)) => {
                assert_eq!(mode, "plan", "effect must carry mode='plan'");
            }
            other => panic!(
                "expected Some(SetPermissionMode(\"plan\")), got {:?}",
                other
            ),
        }
    }

    #[test]
    fn plan_ignores_trailing_args() {
        // Spec's /plan [open] sub-arg is deferred to P0-B.3 #174.
        // For now, any trailing arg is silently ignored (same behavior
        // as the no-arg call). Pins the contract so future work to
        // support [open] is explicit about changing it.
        let (mut ctx, mut rx) = make_bug_ctx();
        PlanHandler
            .execute(&mut ctx, &[String::from("open")])
            .unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1, "trailing arg should not change output");
        assert!(matches!(
            ctx.pending_effect,
            Some(CommandEffect::SetPermissionMode(_))
        ));
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn plan_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("plan") must return Some(handler).
        // Dispatched execute must emit exactly one TextDelta containing
        // "Plan mode" AND stash CommandEffect::SetPermissionMode("plan")
        // on the context — proves plumbing end-to-end.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("plan")
            .expect("plan must be registered in default_registry()");

        let (mut ctx, mut rx) = make_bug_ctx();
        handler
            .execute(&mut ctx, &[])
            .expect("dispatched /plan must not error");

        // Assertion 1: TextDelta emitted.
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "expected exactly one TextDelta; got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::TextDelta(s) => {
                assert!(
                    s.to_lowercase().contains("plan mode"),
                    "TextDelta must contain 'plan mode' (case-insensitive); got: {}",
                    s
                );
            }
            other => panic!("expected TextDelta, got {:?}", other),
        }

        // Assertion 2: SetPermissionMode("plan") effect stashed.
        match ctx.pending_effect {
            Some(CommandEffect::SetPermissionMode(ref mode)) => {
                assert_eq!(mode, "plan", "effect must carry mode='plan'");
            }
            other => panic!(
                "expected Some(SetPermissionMode(\"plan\")), got {:?}",
                other
            ),
        }
    }
}
