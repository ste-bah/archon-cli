//! TASK-AGS-807: /status slash-command handler (body-migrate target).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!` stub
//! in `src/command/registry.rs` and the legacy match arm at
//! `src/command/slash.rs:342-380`. The legacy body read four
//! `tokio::sync::Mutex` guards via `.lock().await`. Because
//! `CommandHandler::execute` is SYNC (Q1=A invariant), we cannot await in
//! `execute`. Solution: the dispatch site at `slash.rs` builds a
//! [`StatusSnapshot`] by awaiting the locks BEFORE calling
//! `Dispatcher::dispatch`, threads it through [`CommandContext`], and the
//! sync handler consumes owned values.
//!
//! Aliases: `[info]` (per spec REQ-FOR-D7 validation criterion 2 and
//! TASK-AGS-807.md). The shipped stub used `[stat]`; orchestrator Q/A
//! approved the spec alias `info` for this ticket.

use std::sync::atomic::Ordering;

use archon_llm::effort::EffortLevel;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;

/// Owned snapshot of every value the /status body reads from shared
/// state. Built at the dispatch site (where `.await` is allowed) and
/// threaded through `CommandContext` so the sync handler can consume
/// without holding locks.
///
/// All fields are plain owned types — no `Arc`, no `Mutex`, no borrows.
#[derive(Debug, Clone)]
pub(crate) struct StatusSnapshot {
    pub(crate) current_model: String,
    pub(crate) perm_mode: String,
    pub(crate) fast_mode: bool,
    pub(crate) effort: EffortLevel,
    pub(crate) thinking_visible: bool,
    pub(crate) session_id_short: String,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) turn_count: u64,
}

/// Build a [`StatusSnapshot`] by awaiting the four async locks in the
/// SAME ORDER as the shipped `/status` body (session_stats →
/// model_override_shared → permission_mode → effort_level_shared) plus
/// the two atomics (fast_mode_shared, show_thinking).
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/status` (or its alias `/info`). All other commands leave
/// `status_snapshot = None` to avoid unnecessary lock traffic.
pub(crate) async fn build_status_snapshot(
    slash_ctx: &SlashCommandContext,
) -> StatusSnapshot {
    // Lock order preserved from shipped body at slash.rs:342-380.
    let stats = slash_ctx.session_stats.lock().await;
    let current_model = {
        let ov = slash_ctx.model_override_shared.lock().await;
        if ov.is_empty() {
            slash_ctx.default_model.clone()
        } else {
            ov.clone()
        }
    };
    let perm_mode = slash_ctx.permission_mode.lock().await;
    let effort = slash_ctx.effort_level_shared.lock().await;

    let fast = slash_ctx.fast_mode_shared.load(Ordering::Relaxed);
    let thinking_visible = slash_ctx.show_thinking.load(Ordering::Relaxed);

    let sid = &slash_ctx.session_id[..8.min(slash_ctx.session_id.len())];

    StatusSnapshot {
        current_model,
        perm_mode: perm_mode.clone(),
        fast_mode: fast,
        effort: *effort,
        thinking_visible,
        session_id_short: sid.to_string(),
        input_tokens: stats.input_tokens,
        output_tokens: stats.output_tokens,
        turn_count: stats.turn_count,
    }
    // Guards drop here — locks released before return.
}

/// Zero-sized handler registered as the primary `/status` command.
/// Aliases: `[info]`.
pub(crate) struct StatusHandler;

impl CommandHandler for StatusHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // Defensive: build_command_context is responsible for populating
        // status_snapshot when the primary resolves to /status. A None
        // here indicates a wiring regression (e.g. the builder was
        // bypassed or the alias map drifted), not a user-facing error —
        // but we surface it as an anyhow::Error so the bug is loud rather
        // than silent.
        let snap = ctx.status_snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "StatusHandler invoked without status_snapshot populated \
                 — build_command_context bug"
            )
        })?;

        let in_k = snap.input_tokens as f64 / 1000.0;
        let out_k = snap.output_tokens as f64 / 1000.0;
        let thinking_str = if snap.thinking_visible {
            "visible"
        } else {
            "hidden"
        };
        let fast_label = if snap.fast_mode { "on" } else { "off" };

        // Byte-for-byte faithful to shipped body at slash.rs:342-380.
        // Case of labels is preserved: "Model:", "Mode:", "Fast mode:",
        // "Effort:", "Thinking:", "Session:", "Tokens:", "Turns:".
        let msg = format!(
            "\n\
             Model: {current_model}\n\
             Mode: {perm_mode} (permissions)\n\
             Fast mode: {fast_label}\n\
             Effort: {effort}\n\
             Thinking: {thinking_str}\n\
             Session: {sid}\n\
             Tokens: {in_k:.1}k in / {out_k:.1}k out\n\
             Turns: {turns}\n",
            current_model = snap.current_model,
            perm_mode = snap.perm_mode,
            effort = snap.effort,
            sid = snap.session_id_short,
            turns = snap.turn_count,
        );

        // Sync try_send analogous to /tasks precedent (AGS-806). Channel
        // full/closed is best-effort — dropping a status line under
        // backpressure is preferable to stalling the input pipeline.
        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(msg));
        Ok(())
    }

    fn description(&self) -> &str {
        "Show session status (model, effort, token use)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["info"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-807: tests for /status slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Minimal test-only StatusSnapshot used by handler tests. Values
    /// are chosen so format-string substitutions are obvious in
    /// assertion output (e.g. 1234 tokens → "1.2k in").
    fn fixture_snapshot() -> StatusSnapshot {
        StatusSnapshot {
            current_model: "claude-opus-4-7".to_string(),
            perm_mode: "default".to_string(),
            fast_mode: false,
            effort: EffortLevel::Medium,
            thinking_visible: false,
            session_id_short: "abcd1234".to_string(),
            input_tokens: 1234,
            output_tokens: 567,
            turn_count: 3,
        }
    }

    /// Build a `CommandContext` with a freshly-created channel and the
    /// supplied (optional) snapshot. Tests that do not exercise the
    /// snapshot path pass `None`.
    fn make_ctx(
        snapshot: Option<StatusSnapshot>,
    ) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = mpsc::channel::<TuiEvent>(16);
        (
            CommandContext {
                tui_tx: tx,
                status_snapshot: snapshot,
                // TASK-AGS-808: /status tests never exercise /model
                // paths — None on both new fields.
                model_snapshot: None,
                // TASK-AGS-809: /status tests never exercise /cost
                // paths — None.
                cost_snapshot: None,
                pending_effect: None,
            },
            rx,
        )
    }

    #[test]
    fn status_handler_description_matches() {
        let h = StatusHandler;
        let desc = h.description().to_lowercase();
        assert!(
            desc.contains("status")
                || desc.contains("session")
                || desc.contains("model"),
            "StatusHandler description should reference session/status/model, got: {}",
            h.description()
        );
    }

    #[test]
    fn status_handler_aliases_are_info() {
        let h = StatusHandler;
        assert_eq!(
            h.aliases(),
            &["info"],
            "StatusHandler aliases must be [info] per AGS-807 + spec \
             validation criterion 2 (alias 'info' resolves)"
        );
    }

    #[test]
    fn status_handler_execute_with_snapshot_emits_text_delta_with_model_line() {
        let (mut ctx, mut rx) = make_ctx(Some(fixture_snapshot()));
        let h = StatusHandler;
        h.execute(&mut ctx, &[])
            .expect("StatusHandler::execute must return Ok with snapshot populated");

        let ev = rx.try_recv().expect("must emit a TuiEvent");
        match ev {
            TuiEvent::TextDelta(msg) => {
                assert!(
                    msg.contains("Model: claude-opus-4-7"),
                    "TextDelta must contain 'Model: claude-opus-4-7', got: {msg}"
                );
                assert!(
                    msg.contains("Mode: default (permissions)"),
                    "TextDelta must contain 'Mode: default (permissions)', got: {msg}"
                );
                assert!(
                    msg.contains("Fast mode: off"),
                    "TextDelta must reflect fast_mode=false as 'off', got: {msg}"
                );
                assert!(
                    msg.contains("Thinking: hidden"),
                    "TextDelta must reflect thinking_visible=false as 'hidden', got: {msg}"
                );
                assert!(
                    msg.contains("Session: abcd1234"),
                    "TextDelta must contain session id short, got: {msg}"
                );
                assert!(
                    msg.contains("Turns: 3"),
                    "TextDelta must contain turn count, got: {msg}"
                );
            }
            other => panic!("expected TuiEvent::TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn status_handler_execute_without_snapshot_returns_err() {
        let (mut ctx, _rx) = make_ctx(None);
        let h = StatusHandler;
        let result = h.execute(&mut ctx, &[]);
        assert!(
            result.is_err(),
            "StatusHandler::execute must return Err when status_snapshot is None \
             (defensive: builder bug should surface loudly)"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("status_snapshot")
                || err_msg.contains("build_command_context"),
            "error must describe the missing snapshot, got: {err_msg}"
        );
    }

    #[test]
    fn status_snapshot_round_trip_via_clone() {
        // Cheap sanity check that StatusSnapshot derives Debug + Clone
        // and that cloning preserves all fields (so the type can be
        // inserted into Option<StatusSnapshot> in CommandContext and
        // read back by the handler without needing Copy).
        let snap = fixture_snapshot();
        let cloned = snap.clone();
        assert_eq!(snap.current_model, cloned.current_model);
        assert_eq!(snap.perm_mode, cloned.perm_mode);
        assert_eq!(snap.fast_mode, cloned.fast_mode);
        assert_eq!(snap.thinking_visible, cloned.thinking_visible);
        assert_eq!(snap.session_id_short, cloned.session_id_short);
        assert_eq!(snap.input_tokens, cloned.input_tokens);
        assert_eq!(snap.output_tokens, cloned.output_tokens);
        assert_eq!(snap.turn_count, cloned.turn_count);
        // Debug impl must not panic.
        let _ = format!("{snap:?}");
    }
}
