//! /export command handler.
//!
//! # TASK-AGS-POST-6-EXPORT-MIGRATE (body-migrate via SIDECAR-SLOT)
//!
//! `/export` body migrated from the upstream intercept at former
//! `src/session.rs:2407-2481` into this handler via the SIDECAR-SLOT
//! pattern. EFFECT-SLOT was rejected because
//! `command::context::apply_effect` runs with only `SlashCommandContext`
//! access and is invoked from inside `slash.rs`, which MUST stay
//! zero-diff for this ticket (see the ticket spec's scope guard).
//! /export needs `agent.lock().await` to read
//! `conversation_state().messages`, and the `Arc<tokio::sync::Mutex<
//! Agent>>` only lives in session.rs's input-processor task scope
//! (session.rs:1895).
//!
//! # Data flow
//!
//! ```text
//! user types `/export [format]`
//!   |
//!   v
//! slash.rs::handle_slash_command (unchanged — zero diff)
//!   |
//!   v
//! dispatcher::dispatch -> ExportHandler::execute (SYNC)
//!   - Parse/validate format arg
//!   - On parse failure: emit TuiEvent::Error via try_send, return Ok
//!   - On success: write ExportDescriptor into the shared
//!     pending_export_shared slot (std::sync::Mutex) held on
//!     SlashCommandContext. No .await inside the handler.
//!   |
//!   v
//! slash.rs returns handled=true (unchanged — zero diff)
//!   |
//!   v
//! session.rs `if handled {` drain block (new — one of two session.rs
//! edits this ticket makes):
//!   - .take() the descriptor from the shared slot
//!   - agent.lock().await to read conversation_state().messages
//!   - archon_session::export::export_session(...)
//!   - write file, emit TextDelta/Error events
//!   - then the existing SlashCommandComplete emit runs
//! ```
//!
//! # Why a shared `std::sync::Mutex` rather than a `CommandContext`
//! field
//!
//! The effort-slot precedent (TASK-AGS-POST-6-BODIES-B11-EFFORT) stores
//! `pending_effort_set: Option<EffortLevel>` directly on
//! `CommandContext` and drains it in slash.rs where `__cmd_ctx` is in
//! scope. For /export the drain MUST happen in session.rs (Agent mutex
//! scope) while `__cmd_ctx` is a local inside `handle_slash_command`.
//! A shared `Arc<std::sync::Mutex<Option<ExportDescriptor>>>` held on
//! SlashCommandContext is the one mechanism that lets the sync handler
//! write and session.rs drain without forcing any slash.rs edit.
//! `std::sync::Mutex` (not `tokio::sync::Mutex`) because neither the
//! handler (sync) nor the drain (single `.take()` then release) holds
//! the lock across any `.await`.
//!
//! # Alias preserved
//!
//! `["save"]` preserved from the shipped declare_handler! stub per the
//! AGS-817 /memory precedent (shipped-wins drift-reconcile). The
//! upstream intercept only matched `/export`; `/save` has always
//! reached the handler via the PATH A dispatcher. Under the new flow
//! both `/export` and `/save` route through `ExportHandler::execute`
//! and then through the session.rs drain — the surface behaviour is
//! identical to pre-migration `/export` for both names.

use anyhow::Result;
use archon_session::export::ExportFormat;
use archon_tui::app::TuiEvent;
use std::str::FromStr;

use crate::command::registry::{CommandContext, CommandHandler};

/// SIDECAR-SLOT payload: the parsed `/export` invocation produced by
/// `ExportHandler::execute` and consumed by the session.rs drain.
///
/// Carries the validated `ExportFormat` plus the original arg string
/// for user-facing display (the success message includes the
/// format arg verbatim — preserves pre-migration wording from former
/// session.rs:2454-2463). When the caller typed bare `/export` with no
/// arg, `format_arg_display` is empty; the drain substitutes
/// `"markdown"` for display (byte-identical to the shipped inline
/// ternary).
#[derive(Debug, Clone)]
pub(crate) struct ExportDescriptor {
    /// Parsed `ExportFormat` — `Markdown` when the caller omitted the
    /// arg, otherwise whatever `ExportFormat::from_str` returned.
    pub(crate) format: ExportFormat,
    /// Original trimmed format arg string as typed by the caller, for
    /// inclusion in the success TextDelta. Empty string means the
    /// caller ran bare `/export`; the drain maps that to `"markdown"`
    /// for display.
    pub(crate) format_arg_display: String,
}

/// Zero-sized handler registered as the primary `/export` command.
///
/// Alias: `["save"]` — PRESERVED from the shipped declare_handler!
/// stub. See module rustdoc.
///
/// The handler is sync; it parses the format arg, emits a
/// `TuiEvent::Error` via `try_send` on parse failure (preserves
/// shipped behaviour at former session.rs:2417), otherwise stashes an
/// `ExportDescriptor` in the shared `pending_export_shared` slot. All
/// async I/O (Agent mutex lock, conversation-state read, file write,
/// success/error event emits) lives in the session.rs drain block.
pub(crate) struct ExportHandler;

impl CommandHandler for ExportHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        // Mirror former session.rs:2410-2422 arg parsing: bare
        // `/export` -> Markdown; otherwise parse via from_str and
        // surface parse errors as TuiEvent::Error then return Ok(())
        // without stashing a descriptor (the drain will see None and
        // skip the export body).
        let format_arg = args.first().map(String::as_str).unwrap_or("").trim();
        let format = if format_arg.is_empty() {
            ExportFormat::Markdown
        } else {
            match ExportFormat::from_str(format_arg) {
                Ok(f) => f,
                Err(e) => {
                    // Byte-identical to former session.rs:2417 —
                    // the parse error surfaces the archon_session
                    // error string directly.
                    ctx.emit(TuiEvent::Error(e));
                    return Ok(());
                }
            }
        };

        let desc = ExportDescriptor {
            format,
            format_arg_display: format_arg.to_string(),
        };

        // Stash the descriptor. If the shared slot is absent the
        // handler is running from a test fixture that did not wire up
        // the slot — in that case we surface a loud error so the
        // regression is obvious instead of silently dropping the
        // export. Production always wires the slot via
        // `build_command_context` cloning from SlashCommandContext.
        match &ctx.pending_export {
            Some(slot) => {
                // `std::sync::Mutex` — poisoning here would indicate a
                // prior panic while holding the drain lock. Use
                // `unwrap()` so such a regression surfaces as a panic
                // rather than a silent no-op; the drain holds the lock
                // only across a single `.take()` so poisoning under
                // normal operation is impossible.
                *slot.lock().unwrap() = Some(desc);
            }
            None => {
                ctx.emit(TuiEvent::Error(
                    "export slot not wired on CommandContext; \
                     this is a dispatcher wiring regression — report it"
                        .to_string(),
                ));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub description (shipped-wins drift-reconcile).
        "Export the current session to a file"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Preserved per AGS-817 /memory precedent.
        &["save"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-EXPORT-MIGRATE: tests for the sync /export handler
// (parse + stash path only). The async drain in session.rs is covered
// by the live-smoke-test gate.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel and a
    /// freshly-allocated `Arc<std::sync::Mutex<Option<
    /// ExportDescriptor>>>` shared slot so the handler's stash path
    /// can be observed by the test. Every other optional field stays
    /// `None` — /export does not read any other CommandContext field.
    fn make_ctx() -> (
        CommandContext,
        mpsc::Receiver<TuiEvent>,
        Arc<Mutex<Option<ExportDescriptor>>>,
    ) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        let slot: Arc<Mutex<Option<ExportDescriptor>>> = Arc::new(Mutex::new(None));
        let (ctx, rx) = crate::command::test_support::CtxBuilder::new()
            .with_pending_export(Arc::clone(&slot))
            .build();
        (ctx, rx, slot)
    }

    #[test]
    fn export_handler_description_matches() {
        let h = ExportHandler;
        assert_eq!(
            h.description(),
            "Export the current session to a file",
            "ExportHandler description must match shipped stub verbatim"
        );
    }

    #[test]
    fn export_handler_aliases_preserve_save() {
        let h = ExportHandler;
        assert_eq!(
            h.aliases(),
            &["save"],
            "ExportHandler aliases must preserve 'save' (shipped-wins)"
        );
    }

    #[test]
    fn export_handler_bare_invocation_stashes_markdown_descriptor() {
        let (mut ctx, mut rx, slot) = make_ctx();
        let args: Vec<String> = vec![];
        ExportHandler.execute(&mut ctx, &args).unwrap();

        // No events on the success path — drain owns emission.
        assert!(
            rx.try_recv().is_err(),
            "bare /export must emit zero events from the sync handler"
        );

        let guard = slot.lock().unwrap();
        let desc = guard
            .as_ref()
            .expect("bare /export must stash an ExportDescriptor in the shared slot");
        assert!(
            matches!(desc.format, ExportFormat::Markdown),
            "bare /export must default to Markdown; got {:?}",
            desc.format
        );
        assert_eq!(
            desc.format_arg_display, "",
            "bare /export must stash an empty format_arg_display"
        );
    }

    #[test]
    fn export_handler_json_arg_stashes_json_descriptor() {
        let (mut ctx, mut rx, slot) = make_ctx();
        let args = vec!["json".to_string()];
        ExportHandler.execute(&mut ctx, &args).unwrap();
        assert!(rx.try_recv().is_err());

        let guard = slot.lock().unwrap();
        let desc = guard.as_ref().expect("descriptor stashed");
        assert!(matches!(desc.format, ExportFormat::Json));
        assert_eq!(desc.format_arg_display, "json");
    }

    #[test]
    fn export_handler_text_arg_stashes_text_descriptor() {
        let (mut ctx, mut rx, slot) = make_ctx();
        let args = vec!["text".to_string()];
        ExportHandler.execute(&mut ctx, &args).unwrap();
        assert!(rx.try_recv().is_err());

        let guard = slot.lock().unwrap();
        let desc = guard.as_ref().expect("descriptor stashed");
        assert!(matches!(desc.format, ExportFormat::Text));
        assert_eq!(desc.format_arg_display, "text");
    }

    #[test]
    fn export_handler_markdown_arg_stashes_markdown_descriptor() {
        let (mut ctx, mut rx, slot) = make_ctx();
        let args = vec!["markdown".to_string()];
        ExportHandler.execute(&mut ctx, &args).unwrap();
        assert!(rx.try_recv().is_err());

        let guard = slot.lock().unwrap();
        let desc = guard.as_ref().expect("descriptor stashed");
        assert!(matches!(desc.format, ExportFormat::Markdown));
        assert_eq!(desc.format_arg_display, "markdown");
    }

    #[test]
    fn export_handler_invalid_arg_emits_error_and_stashes_nothing() {
        let (mut ctx, mut rx, slot) = make_ctx();
        let args = vec!["xml".to_string()];
        ExportHandler.execute(&mut ctx, &args).unwrap();

        // The shared slot MUST stay empty — the drain will see None
        // and skip the export body.
        assert!(
            slot.lock().unwrap().is_none(),
            "parse-error branch must NOT stash a descriptor"
        );

        // One TuiEvent::Error with the archon_session error string.
        match rx.try_recv().expect("error event emitted") {
            TuiEvent::Error(_) => {}
            other => panic!("expected TuiEvent::Error, got {other:?}"),
        }
    }

    #[test]
    fn export_handler_missing_slot_emits_error() {
        // Defensive regression test — production always wires the
        // shared slot via build_command_context. If the slot is None
        // the handler must emit a loud error rather than silently
        // drop the export.
        //
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        // `pending_export` is left at the builder default (None) to
        // exercise the wiring-regression branch.
        let (mut ctx, mut rx) = crate::command::test_support::CtxBuilder::new().build();
        ExportHandler.execute(&mut ctx, &[]).unwrap();
        match rx.try_recv().expect("error event emitted") {
            TuiEvent::Error(msg) => {
                assert!(
                    msg.contains("wiring regression"),
                    "missing-slot error must mention wiring regression; \
                     got: {msg}"
                );
            }
            other => panic!("expected TuiEvent::Error, got {other:?}"),
        }
    }
}
