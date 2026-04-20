//! TASK-AGS-818: /export slash-command handler (Option D, CANARY pattern,
//! FOURTH Batch-3 body-migrate — registry hygiene only).
//!
//! # R1 DISPATCH-ORDERING (design)
//!
//! The shipped `/export` body lives in `src/session.rs:2409-2480`, NOT
//! in `src/command/slash.rs`. The TUI input loop in session.rs
//! intercepts `/export` (and `/export <format>`) with a literal string
//! match and runs the body before `handle_slash_command` is ever
//! invoked, because the body needs `agent.lock().await` to read
//! `conversation_state().messages` — a dependency that
//! `CommandContext` does not carry and that the sync
//! `CommandHandler::execute` signature cannot `.await`.
//!
//! Dispatch order under normal operation:
//!
//! ```text
//! session.rs:2409  --> /export intercepted --> `continue` (handler never reached)
//! session.rs:2483  --> handle_slash_command(...)
//! slash.rs         --> Dispatcher::dispatch --> Registry::get("export")
//! export.rs        --> ExportHandler::execute    [UNREACHABLE under normal op]
//! ```
//!
//! If this handler's `execute` DOES fire, the interception path in
//! session.rs has regressed or been removed — the handler's sole job is
//! to emit a diagnostic canary so the bug is loud and traceable instead
//! of silently failing to export.
//!
//! # R2 SCOPE-HELD (real body-migrate deferred)
//!
//! Real body-migrate is deferred to POST-STAGE-6 (ticket
//! `AGS-POST-6-EXPORT`). Completing it requires:
//!
//! 1. Surfacing `Arc<Mutex<Agent>>` (or an effect-slot pattern) through
//!    `CommandContext` so the handler can read
//!    `conversation_state().messages`.
//! 2. Removing the session.rs:2409-2480 interception block without
//!    regressing shipped export behavior.
//! 3. Full re-wire: format arg parsing, export dir creation, filename
//!    generation, and write-path error surfaces.
//!
//! None of that fits Option D's registry-hygiene budget. This ticket
//! explicitly leaves session.rs UNTOUCHED (zero-diff invariant — see
//! R5 below) and reserves the migration for the follow-up.
//!
//! # R3 CANARY-MESSAGE (behavior)
//!
//! The handler emits a deliberately non-help `TextDelta` whose literal
//! byte-for-byte text is:
//!
//! ```text
//! /export is handled by session dispatcher — this message indicates a dispatch ordering bug. Report it.
//! ```
//!
//! This is NOT help text. Do NOT soften it to "use /export with a path"
//! or similar — if an operator ever sees this message, there IS a
//! dispatch-ordering bug and they should report it. Softening the
//! message would mask a real regression.
//!
//! # R4 ALIASES-SHIPPED-WINS
//!
//! Shipped `declare_handler!` stub at registry.rs:513-517 (pre-AGS-818)
//! carried `&["save"]`. Per AGS-817 `/memory` precedent (where
//! `&["mem"]` was preserved as shipped-wins because PATH A dispatcher
//! DOES resolve aliases), this handler preserves `&["save"]`. Dropping
//! the alias would regress any operator workflow depending on `/save`
//! today.
//!
//! Note: `/save` reaches `ExportHandler` via the PATH A dispatcher —
//! NOT via session.rs:2409 (which only matches the literal `/export`
//! prefix). So a user typing `/save` DOES currently bypass the shipped
//! export body and fall through to the canary. This is CORRECT Option D
//! behavior: any invocation that reaches this handler — by any name —
//! signals a dispatch ordering situation worth reporting. The canary
//! message text is accurate for both call paths.
//!
//! # R5 NO-SESSION-RS-DIFF (critical invariant)
//!
//! `git diff HEAD -- src/session.rs | wc -c` MUST be 0 after this
//! ticket lands. Option D's entire premise is that session.rs keeps its
//! zero-diff invariant (held since AGS-805) until POST-STAGE-6 does the
//! real migration. Any edit to session.rs in this ticket — even a
//! comment tweak — violates Option D and must be reverted.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Byte-for-byte canary message emitted whenever `ExportHandler::execute`
/// fires. Extracted to a module-level const so tests can pin the exact
/// string without duplicating it.
///
/// See R3 in the module rustdoc — this text is deliberately NOT help
/// text and must not be softened.
const CANARY_MESSAGE: &str = "/export is handled by session dispatcher \
— this message indicates a dispatch ordering bug. Report it.";

/// Zero-sized handler registered as the primary `/export` command.
///
/// Alias: `["save"]` — PRESERVED from the shipped declare_handler!
/// stub (shipped-wins drift-reconcile; see R4 in module rustdoc).
///
/// Under normal operation this handler is UNREACHABLE via `/export`
/// because session.rs:2409 intercepts first with `continue`. The
/// `/save` alias reaches the handler through the PATH A dispatcher
/// (session.rs does not match `/save`). In either case, arriving here
/// signals the canary message to the operator. Args are ignored —
/// format/path parsing belongs to the real body-migrate (POST-STAGE-6).
pub(crate) struct ExportHandler;

impl CommandHandler for ExportHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // Canary emit — byte-for-byte `CANARY_MESSAGE`. `try_send` is
        // sync and legal inside the non-async `execute` signature.
        // Best-effort drop semantics match AGS-812..817 handler peers.
        let _ = ctx
            .tui_tx
            .try_send(TuiEvent::TextDelta(CANARY_MESSAGE.to_string()));
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:513-517 (shipped-wins drift-reconcile).
        "Export the current session to a file"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Preserved per AGS-817 /memory precedent. See R4 in module
        // rustdoc.
        &["save"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-818: tests for /export slash-command canary handler
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel.
    /// /export is a CANARY-pattern handler — no snapshot, no effect
    /// slot, no extra context field — so every optional field stays
    /// `None`. Mirrors the make_ctx fixtures in voice.rs / memory.rs.
    fn make_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = mpsc::channel::<TuiEvent>(16);
        (
            CommandContext {
                tui_tx: tx,
                status_snapshot: None,
                model_snapshot: None,
                cost_snapshot: None,
                mcp_snapshot: None,
                context_snapshot: None,
                session_id: None,
                memory: None,
                garden_config: None,
                fast_mode_shared: None,
                // TASK-AGS-POST-6-BODIES-B02-THINKING: /export tests never exercise /thinking paths — None.
                show_thinking: None,
                // TASK-AGS-POST-6-BODIES-B04-DIFF: /export tests never exercise /diff paths — None.
                working_dir: None,
                // TASK-AGS-POST-6-BODIES-B06-HELP: /export tests never exercise /help paths — None.
                skill_registry: None,
                // TASK-AGS-POST-6-BODIES-B08-DENIALS: /export tests never exercise /denials paths — None.
                denial_snapshot: None,
                effort_snapshot: None,
                permissions_snapshot: None,
                copy_snapshot: None,
                doctor_snapshot: None,
                usage_snapshot: None,
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    #[test]
    fn export_handler_description_matches() {
        let h = ExportHandler;
        assert_eq!(
            h.description(),
            "Export the current session to a file",
            "ExportHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    #[test]
    fn export_handler_aliases_preserve_save() {
        let h = ExportHandler;
        assert_eq!(
            h.aliases(),
            &["save"],
            "ExportHandler aliases must preserve 'save' from the shipped \
             declare_handler! stub (shipped-wins drift-reconcile per \
             AGS-817 /memory precedent)"
        );
    }

    #[test]
    fn export_handler_execute_emits_canary_text_delta() {
        let (mut ctx, mut rx) = make_ctx();
        let h = ExportHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "ExportHandler::execute must return Ok(()), got: {res:?}"
        );

        let ev = rx
            .try_recv()
            .expect("ExportHandler::execute must emit one event");
        match ev {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, CANARY_MESSAGE,
                    "ExportHandler::execute must emit the canary \
                     message byte-for-byte (see R3 in module rustdoc)"
                );
            }
            other => panic!(
                "ExportHandler::execute must emit TextDelta, got: {other:?}"
            ),
        }
    }

    #[test]
    fn export_handler_execute_canary_emitted_regardless_of_args() {
        // Args are ignored — format/path parsing is SCOPE-HELD to
        // POST-STAGE-6 real body-migrate. Every arg shape must produce
        // the same canary message byte-for-byte.
        let arg_shapes: &[&[&str]] = &[
            &["markdown"],
            &["--format=json"],
            &["/tmp/test.md"],
        ];
        for shape in arg_shapes {
            let (mut ctx, mut rx) = make_ctx();
            let h = ExportHandler;
            let args: Vec<String> =
                shape.iter().map(|s| s.to_string()).collect();
            let res = h.execute(&mut ctx, &args);
            assert!(
                res.is_ok(),
                "ExportHandler::execute(args={shape:?}) must return \
                 Ok(()), got: {res:?}"
            );
            let ev = rx.try_recv().expect(
                "ExportHandler::execute must emit one event regardless \
                 of args",
            );
            match ev {
                TuiEvent::TextDelta(text) => {
                    assert_eq!(
                        text, CANARY_MESSAGE,
                        "ExportHandler::execute(args={shape:?}) must \
                         emit the canary byte-for-byte — args are \
                         ignored by Option D"
                    );
                }
                other => panic!(
                    "ExportHandler::execute(args={shape:?}) must emit \
                     TextDelta, got: {other:?}"
                ),
            }
        }
    }
}
