//! TASK-AGS-808: /model slash-command handler (body-migrate target).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!` stub
//! in `src/command/registry.rs` and the legacy match arm at
//! `src/command/slash.rs:146-180`. The legacy body had TWO sides:
//!
//! * READ side (no args): display the current model by locking
//!   `slash_ctx.model_override_shared` via `.lock().await`.
//! * WRITE side (arg provided): validate the input and overwrite
//!   `*slash_ctx.model_override_shared.lock().await`.
//!
//! `CommandHandler::execute` is SYNC (Q1=A invariant) so NEITHER of those
//! `.await` calls is legal inside `execute`. Two complementary patterns
//! bridge the gap:
//!
//! 1. READ path — per-ticket [`ModelSnapshot`] populated by
//!    `build_command_context` at the dispatch site BEFORE
//!    `Dispatcher::dispatch` (same shape as AGS-807 for `/status`).
//! 2. WRITE path — new [`CommandEffect`] enum
//!    (`crate::command::registry::CommandEffect`). The sync handler
//!    stashes a variant into `CommandContext::pending_effect`; after
//!    dispatch returns, `slash.rs::handle_slash_command` calls
//!    `command::context::apply_effect` which awaits the mutex write.
//!
//! Aliases: `[m, switch-model]` per spec.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};
use crate::slash_context::SlashCommandContext;

/// Owned snapshot of the single value the /model READ path needs from
/// shared state. Built at the dispatch site (where `.await` is allowed)
/// and threaded through [`CommandContext`] so the sync handler can
/// consume without holding locks.
///
/// Field is a plain owned `String` — no `Arc`, no `Mutex`, no borrow.
#[derive(Debug, Clone)]
pub(crate) struct ModelSnapshot {
    /// The resolved current model name: the override if non-empty,
    /// otherwise the configured default. Matches the shipped READ body's
    /// `if ov.is_empty() { default_model } else { ov }` selection.
    pub(crate) current_model: String,
}

/// Build a [`ModelSnapshot`] by awaiting the `model_override_shared`
/// lock in the SAME order and with the SAME selection logic as the
/// shipped READ path at `src/command/slash.rs:146-180`.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/model` (or one of its aliases `/m` / `/switch-model`).
/// All other commands leave `model_snapshot = None` to avoid unnecessary
/// lock traffic.
pub(crate) async fn build_model_snapshot(
    slash_ctx: &SlashCommandContext,
) -> ModelSnapshot {
    let ov = slash_ctx.model_override_shared.lock().await;
    let current_model = if ov.is_empty() {
        slash_ctx.default_model.clone()
    } else {
        ov.clone()
    };
    ModelSnapshot { current_model }
    // Guard drops here — lock released before return.
}

/// Zero-sized handler registered as the primary `/model` command.
/// Aliases: `[m, switch-model]`.
pub(crate) struct ModelHandler;

impl CommandHandler for ModelHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // Shipped body uses `s.strip_prefix("/model").unwrap_or("").trim()`
        // which reduces to a single free-form trailing string. The
        // dispatcher hands us parser-tokenized `args: &[String]`. Joining
        // with " " and trimming reproduces the shipped selection for the
        // one-token case `/model opus` and is stable for any
        // hypothetical multi-token future (e.g. flags). Whitespace-only
        // rejoin collapses back to the empty string.
        let arg_str = args.join(" ").trim().to_string();

        if arg_str.is_empty() {
            // READ path: the builder must have populated the snapshot
            // when the primary resolved to `/model`. A `None` here
            // indicates a wiring regression (builder bypassed or alias
            // map drifted); surface it as a loud `Err` rather than a
            // user-facing message.
            let snap = ctx.model_snapshot.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "ModelHandler invoked without model_snapshot populated \
                     — build_command_context bug"
                )
            })?;

            // Byte-for-byte faithful to shipped READ body at
            // slash.rs:158-162. Output is a TextDelta (no view opened).
            let msg = format!(
                "\nCurrent model: {current}\n\
                 Usage: /model <name>\n\
                 Shortcuts: opus, sonnet, haiku\n",
                current = snap.current_model,
            );
            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(msg));
            return Ok(());
        }

        // WRITE path: validate, then (on Ok) stash the effect + emit
        // ModelChanged + TextDelta. On Err emit TuiEvent::Error and do
        // NOT stash any effect.
        match archon_tools::validation::validate_model_name(&arg_str) {
            Ok(resolved) => {
                // Sync slot-write: the actual `model_override_shared`
                // mutex write is performed by `apply_effect` in
                // `command::context` after dispatch returns. That is
                // where `.await` is legal.
                ctx.pending_effect =
                    Some(CommandEffect::SetModelOverride(resolved.clone()));
                let _ = ctx
                    .tui_tx
                    .try_send(TuiEvent::ModelChanged(resolved.clone()));
                let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(format!(
                    "\nModel switched to {resolved}.\n"
                )));
            }
            Err(msg) => {
                let _ = ctx.tui_tx.try_send(TuiEvent::Error(msg));
            }
        }
        Ok(())
    }

    fn description(&self) -> &str {
        "Show or switch the active model"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["m", "switch-model"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-808: tests for /model slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::test_support::*;

    #[test]
    fn model_handler_description_matches() {
        let h = ModelHandler;
        let desc = h.description();
        assert!(!desc.is_empty(), "ModelHandler description must be non-empty");
        assert!(
            desc.to_lowercase().contains("model"),
            "ModelHandler description should reference 'model', got: {desc}"
        );
    }

    #[test]
    fn model_handler_aliases_are_m_and_switch_model() {
        let h = ModelHandler;
        assert_eq!(
            h.aliases(),
            &["m", "switch-model"],
            "ModelHandler aliases must be [m, switch-model] per AGS-808 spec"
        );
    }

    #[test]
    fn model_handler_execute_no_args_emits_current_model_text() {
        let snap = ModelSnapshot {
            current_model: "opus".to_string(),
        };
        let (mut ctx, mut rx) = make_model_ctx(Some(snap));
        let h = ModelHandler;
        h.execute(&mut ctx, &[])
            .expect("ModelHandler::execute must return Ok with snapshot populated");

        let ev = rx.try_recv().expect("must emit a TuiEvent");
        match ev {
            TuiEvent::TextDelta(msg) => {
                assert!(
                    msg.contains("Current model: opus"),
                    "TextDelta must contain 'Current model: opus', got: {msg}"
                );
                assert!(
                    msg.contains("Usage: /model <name>"),
                    "TextDelta must contain the usage line, got: {msg}"
                );
                assert!(
                    msg.contains("Shortcuts: opus, sonnet, haiku"),
                    "TextDelta must contain shortcuts line, got: {msg}"
                );
            }
            other => panic!("expected TuiEvent::TextDelta, got {other:?}"),
        }
        // READ path must NOT stash a CommandEffect (nothing to apply).
        assert!(
            ctx.pending_effect.is_none(),
            "READ path must not produce a CommandEffect"
        );
    }

    #[test]
    fn model_handler_execute_no_args_without_snapshot_returns_err() {
        let (mut ctx, _rx) = make_model_ctx(None);
        let h = ModelHandler;
        let result = h.execute(&mut ctx, &[]);
        assert!(
            result.is_err(),
            "ModelHandler::execute must return Err when model_snapshot is None \
             (defensive: builder bug should surface loudly)"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("model_snapshot")
                || err_msg.contains("build_command_context"),
            "error must describe the missing snapshot, got: {err_msg}"
        );
    }

    #[test]
    fn model_handler_execute_with_valid_arg_sets_effect_and_emits_events() {
        // snapshot not needed for WRITE path, pass None.
        let (mut ctx, mut rx) = make_model_ctx(None);
        let h = ModelHandler;
        h.execute(&mut ctx, &["opus".to_string()])
            .expect("valid arg must produce Ok(())");

        // validate_model_name("opus") resolves to "claude-opus-4-6"
        // (see crates/archon-tools/src/validation.rs KNOWN_SHORTCUTS).
        let expected = "claude-opus-4-6".to_string();
        match ctx.pending_effect.as_ref() {
            Some(CommandEffect::SetModelOverride(s)) => {
                assert_eq!(
                    s, &expected,
                    "pending_effect must carry the resolved full model id"
                );
            }
            None => panic!("WRITE path must stash a CommandEffect::SetModelOverride"),
        }

        // Collect events in emission order.
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        // Expect at least ModelChanged + TextDelta ("Model switched...").
        let mut saw_model_changed = false;
        let mut saw_text_delta = false;
        for ev in &events {
            match ev {
                TuiEvent::ModelChanged(s) => {
                    assert_eq!(s, &expected);
                    saw_model_changed = true;
                }
                TuiEvent::TextDelta(msg) => {
                    if msg.contains("Model switched to claude-opus-4-6") {
                        saw_text_delta = true;
                    }
                }
                _ => {}
            }
        }
        assert!(
            saw_model_changed,
            "WRITE path must emit TuiEvent::ModelChanged"
        );
        assert!(
            saw_text_delta,
            "WRITE path must emit TuiEvent::TextDelta with 'Model switched to ...'"
        );
    }

    #[test]
    fn model_handler_execute_with_invalid_arg_emits_error_no_effect() {
        let (mut ctx, mut rx) = make_model_ctx(None);
        let h = ModelHandler;
        h.execute(&mut ctx, &["definitely-not-a-model-xyz".to_string()])
            .expect("invalid arg path still returns Ok(()) — error is emitted as event");

        assert!(
            ctx.pending_effect.is_none(),
            "invalid WRITE must NOT stash an effect"
        );

        let ev = rx
            .try_recv()
            .expect("invalid arg must emit a TuiEvent::Error");
        match ev {
            TuiEvent::Error(msg) => {
                assert!(
                    !msg.is_empty(),
                    "Error message must be non-empty, got empty"
                );
            }
            other => panic!("expected TuiEvent::Error, got {other:?}"),
        }
    }
}
