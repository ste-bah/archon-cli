//! TASK-AGS-POST-6-BODIES-B04-DIFF: /diff slash-command handler
//! (Option C, DIRECT with-effect pattern body-migrate).
//!
//! Reference: docs/stage-7.5/tickets/TASK-AGS-POST-6-BODIES-B04-DIFF.md
//! Based on: src/command/slash.rs:961 (live `handle_diff_command` helper)
//! Source: src/command/registry.rs:673 (`declare_handler!(DiffHandler, ...)`
//!   no-op stub being replaced)
//! Derived from: src/command/model.rs (TASK-AGS-808 effect-slot precedent —
//!   sync handler stashes `CommandEffect`, dispatch-site `apply_effect`
//!   awaits the mutation).
//!
//! Gate 2 real impl. Replaces the Gate 1 `#[cfg(test)] mod tests` skeleton
//! (5 `#[ignore]` + `todo!()` stubs) with a live `impl CommandHandler for
//! DiffHandler` and 5 de-ignored tests that pin the effect-stash contract,
//! the missing-working_dir error contract, the trailing-args policy, the
//! byte-identity `description()` contract, and the zero-direct-events
//! contract.
//!
//! # Why DIRECT with-effect (not trivial, not snapshot)
//!
//! The pre-migration `/diff` body (deleted by Gate 3; see
//! slash.rs:362-374 breadcrumb) was:
//!
//! ```ignore
//! "/diff" => {
//!     handle_diff_command(tui_tx, &ctx.working_dir).await;
//!     true
//! }
//! ```
//!
//! where `handle_diff_command` (slash.rs:961) spawns `git diff --stat`
//! via `tokio::process::Command::new("git")` and emits up to five
//! different `TuiEvent` variants depending on exit status and stdout
//! content (not-a-repo TextDelta, git-error Error, spawn-error Error,
//! no-changes TextDelta, stdout TextDelta).
//!
//! - Shipped behavior REQUIRES `await` on a subprocess, which cannot
//!   run inside a sync `CommandHandler::execute`.
//! - Therefore the handler MUST stash a `CommandEffect` variant; the
//!   dispatch site in `slash.rs::handle_slash_command` consumes it via
//!   `.take()` and `apply_effect` awaits the subprocess call — the
//!   established TASK-AGS-808 effect-slot pattern (see `model.rs`).
//! - Unlike `/model` (which stashes a resolved String to write into a
//!   mutex), `/diff` stashes a `PathBuf` (the current working directory)
//!   to pass into the existing `handle_diff_command` helper unchanged.
//! - Because the handler needs the project root but `CommandContext`
//!   previously had no `working_dir` field, B04 adds `working_dir:
//!   Option<PathBuf>` to `CommandContext` and populates it
//!   UNCONDITIONALLY in `build_command_context` from
//!   `SlashCommandContext::working_dir` (mirrors the AGS-815
//!   `session_id` / AGS-817 `memory` / B01 `fast_mode_shared` / B02
//!   `show_thinking` cross-cutting precedent).
//!
//! # Byte-for-byte output preservation
//!
//! The handler itself emits NOTHING in the happy path — it only stashes
//! the effect. All user-visible output (5 emission branches) continues
//! to be produced by the existing `handle_diff_command` helper at
//! slash.rs:961, which `apply_effect` calls unchanged. Byte-identity of
//! the emitted TextDelta / Error strings is therefore preserved by
//! CALL-SITE REUSE, not by literal duplication in this module.
//!
//! `description()` string IS reproduced in this module (replacing the
//! `declare_handler!` macro arg) and is byte-identical to the shipped
//! `"Show a diff of recent file modifications"` at the former
//! registry.rs:673 stub (MD5 `c452e46307d9621006ffff5ceeaadd02`).
//!
//! # Trailing-args policy (decision rationale)
//!
//! The pre-migration arm (at former slash.rs:356, deleted by Gate 3)
//! matched exactly `"/diff"` and would have fallen through for
//! `"/diff foo"` to the default "unknown command" handler.
//! Post-migration, ALL `/diff*` inputs route to `DiffHandler`
//! via the registry. Chosen preservation strategy: **ignore trailing
//! args and always stash the RunGitDiffStat effect** — mirrors the
//! B03-BUG trailing-args promotion. Simpler code, better UX. The
//! Gate 2 test `diff_handler_ignores_trailing_args` pins this contract.
//!
//! # Missing `working_dir` handling
//!
//! Test fixtures that construct a `CommandContext` without a full
//! `SlashCommandContext` (via `make_*_ctx` helpers in
//! `test_support.rs`) may leave `working_dir: None`. When the handler
//! sees `None`, it emits a `TuiEvent::Error` with a missing-shared-
//! state message (adapts the B01-FAST `fast_mode_shared: None` Err
//! pattern into an event emission so `execute` stays `Ok(())` — keeps
//! the dispatcher contract uniform) and returns `Ok(())` without
//! stashing an effect. Production code paths always populate
//! `Some(path)` in `build_command_context`, so this branch is reachable
//! only via tests.

use std::path::PathBuf;

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};

/// Zero-sized handler registered as the primary `/diff` command.
///
/// No aliases. Shipped pre-B04-DIFF stub carried none; spec lists none.
/// Trailing args are intentionally ignored (see module rustdoc
/// "Trailing-args policy").
pub(crate) struct DiffHandler;

impl CommandHandler for DiffHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // Happy path: working_dir populated by build_command_context.
        // Stash the effect synchronously; apply_effect will await the
        // subprocess call via the existing LIVE handle_diff_command
        // helper at slash.rs:961 (byte-identical TextDelta/Error
        // strings preserved by call-site reuse).
        //
        // Trailing args intentionally ignored (see module rustdoc):
        // `_args` underscore-prefixed to signal the unused binding.
        // Mirrors B03-BUG trailing-args promotion policy.
        match &ctx.working_dir {
            Some(path) => {
                // Clone the PathBuf into the effect variant so the
                // effect carries owned data across the .take() boundary
                // at slash.rs:51 — no borrow on ctx lifetime.
                ctx.pending_effect =
                    Some(CommandEffect::RunGitDiffStat(PathBuf::from(path)));
            }
            None => {
                // Adapts the B01-FAST / B02-THINKING `None`-sentinel
                // pattern: emit a TuiEvent::Error describing the
                // missing-shared-state condition. `execute` stays
                // `Ok(())` so the dispatcher contract is uniform
                // across all DIRECT handlers.
                let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                    "DiffHandler: working_dir not populated in CommandContext".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn description(&self) -> &str {
        // Byte-identical to the shipped registry.rs:673 stub
        // description — preserves shipped-wins drift-reconcile.
        // MD5 c452e46307d9621006ffff5ceeaadd02 verified at Sherlock
        // Gate 3.
        "Show a diff of recent file modifications"
    }
}

#[cfg(test)]
mod tests {
    // Gate 2 real tests. Replace the Gate 1 `#[ignore]` + `todo!()`
    // skeleton with real assertions against the landed DiffHandler impl,
    // the new `CommandContext::working_dir` field, and the new
    // `CommandEffect::RunGitDiffStat(PathBuf)` variant. Uses the
    // `make_diff_ctx` helper added to `test_support.rs` in this gate.
    // Test names preserved from Gate 1 skeleton for traceability.

    use super::*;
    use crate::command::registry::{CommandEffect, CommandHandler};
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;
    use std::path::PathBuf;

    #[test]
    fn diff_handler_stashes_effect_when_working_dir_present() {
        let path = PathBuf::from("/tmp");
        let (mut ctx, mut rx) = make_diff_ctx(Some(path.clone()));
        DiffHandler.execute(&mut ctx, &[]).unwrap();
        match ctx.pending_effect {
            Some(CommandEffect::RunGitDiffStat(ref p)) => {
                assert_eq!(
                    p, &path,
                    "RunGitDiffStat must carry the cloned working_dir PathBuf"
                );
            }
            other => panic!(
                "expected Some(CommandEffect::RunGitDiffStat(path)), got: {:?}",
                other
            ),
        }
        let events = drain_tui_events(&mut rx);
        assert!(
            events.is_empty(),
            "happy path must emit zero events directly (all output \
             deferred through apply_effect -> handle_diff_command); \
             got: {:?}",
            events
        );
    }

    #[test]
    fn diff_handler_errors_when_working_dir_none() {
        let (mut ctx, mut rx) = make_diff_ctx(None);
        DiffHandler.execute(&mut ctx, &[]).unwrap();
        assert!(
            ctx.pending_effect.is_none(),
            "None-working_dir path must NOT stash an effect; got: {:?}",
            ctx.pending_effect
        );
        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "None-working_dir path must emit exactly one event; got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::Error(msg) => {
                assert!(
                    msg.contains("working_dir"),
                    "expected Error message to describe missing working_dir; \
                     got: {:?}",
                    msg
                );
            }
            other => panic!(
                "expected TuiEvent::Error for None-working_dir path, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn diff_handler_ignores_trailing_args() {
        let path = PathBuf::from("/tmp");
        let (mut ctx_a, _rx_a) = make_diff_ctx(Some(path.clone()));
        let (mut ctx_b, _rx_b) = make_diff_ctx(Some(path.clone()));
        DiffHandler.execute(&mut ctx_a, &[]).unwrap();
        DiffHandler
            .execute(&mut ctx_b, &[String::from("foo")])
            .unwrap();
        // Both must stash RunGitDiffStat with the same PathBuf —
        // trailing arg has no effect on the stashed variant.
        match (&ctx_a.pending_effect, &ctx_b.pending_effect) {
            (
                Some(CommandEffect::RunGitDiffStat(pa)),
                Some(CommandEffect::RunGitDiffStat(pb)),
            ) => {
                assert_eq!(
                    pa, pb,
                    "trailing args must not change the stashed PathBuf — \
                     must be byte-identical to args=[] case"
                );
                assert_eq!(
                    pa, &path,
                    "stashed PathBuf must equal the working_dir supplied \
                     to the fixture"
                );
            }
            (a, b) => panic!(
                "expected both contexts to stash \
                 Some(CommandEffect::RunGitDiffStat(path)); got a={:?}, b={:?}",
                a, b
            ),
        }
    }

    #[test]
    fn diff_handler_description_byte_identical_to_shipped() {
        // Byte-identical to the shipped registry.rs:673 stub
        // description literal. MD5 c452e46307d9621006ffff5ceeaadd02.
        // Sherlock Gate 3 will re-verify the MD5 against the code.
        assert_eq!(
            DiffHandler.description(),
            "Show a diff of recent file modifications",
            "description() must be byte-identical to the shipped \
             declare_handler! macro arg"
        );
    }

    #[test]
    fn diff_handler_execute_emits_no_events_directly() {
        // Explicit zero-direct-events contract: all user-visible
        // output is deferred through apply_effect ->
        // handle_diff_command. The execute path is effect-only in the
        // happy case.
        let path = PathBuf::from("/tmp");
        let (mut ctx, mut rx) = make_diff_ctx(Some(path));
        DiffHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert!(
            events.is_empty(),
            "execute() must emit zero direct TuiEvents in the happy \
             path (Some-working_dir); all output deferred through \
             apply_effect -> handle_diff_command; got: {:?}",
            events
        );
    }
}
