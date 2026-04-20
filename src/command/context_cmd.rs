//! TASK-AGS-814: /context slash-command handler (body-migrate target,
//! SNAPSHOT-ONLY pattern reuse).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!`
//! stub in `src/command/registry.rs:447-451` and the legacy match arm
//! at `src/command/slash.rs:267-331`. Fifth Batch-2 SNAPSHOT migration
//! (after AGS-807 /status, AGS-809 /cost, AGS-811 /mcp).
//!
//! # File name (R-item NAMING)
//!
//! This file is named `context_cmd.rs` — NOT `context.rs` — because the
//! path `src/command/context.rs` is already occupied by the
//! `build_command_context` / `apply_effect` dispatch-site helper that
//! every body-migrate ticket references. Naming the per-command handler
//! `context_cmd.rs` avoids a file collision without having to rename
//! the builder module (which is referenced from `slash.rs` and every
//! prior body-migrate's rustdoc).
//!
//! # Why SNAPSHOT-ONLY (no effect slot)?
//!
//! The shipped /context body is READ-ONLY — it acquires a single
//! `tokio::sync::Mutex` guard on `ctx.session_stats`, reads the three
//! counters (`input_tokens`, `output_tokens`, `turn_count`), reads the
//! two `Copy` usize fields `system_prompt_chars` + `tool_defs_chars`
//! off `SlashCommandContext` directly, and emits a formatted text
//! delta. There are no writes back to `SlashCommandContext` state.
//!
//! Because `CommandHandler::execute` is SYNC (Q1=A invariant), the
//! `.await` call on `session_stats.lock()` is not legal inside
//! `execute`. Solution (same snapshot pattern as AGS-807/809/811): the
//! dispatch site at `slash.rs` (via `build_command_context`) acquires
//! the guard BEFORE calling `Dispatcher::dispatch`, copies the three
//! counters into an owned [`ContextSnapshot`], and threads the owned
//! values through [`CommandContext`] so the sync handler consumes
//! without holding any async-mutex guard.
//!
//! /context is READ-ONLY — there is no `CommandEffect` variant for
//! this ticket.
//!
//! # Byte-for-byte output preservation
//!
//! Every emitted value mirrors the deleted slash.rs:267-331 body:
//! - `context_limit` = 200_000.0 (200k token budget).
//! - `bar_width` = 40 chars, filled with `#` and padded with `-`.
//! - Percent formatted as `{pct:.1}%`, clamped to 100.0.
//! - `fmt_tok` helper: thousand-suffix `{:.1}k` or raw `{:.0}` digits.
//! - `fixed_overhead = sys_prompt_tokens + tool_def_tokens` (~4 chars
//!   per token).
//! - `conversation_tokens = max(input_tokens, fixed_overhead) -
//!   fixed_overhead` when `input_tokens > 0`, else 0.0.
//! - `total_context = fixed_overhead + conversation_tokens`.
//! - `input_k` / `output_k` = raw tokens / 1000.0.
//! - Event variant: `TuiEvent::TextDelta(msg)` — unchanged.
//!
//! The one emission-primitive change is `tui_tx.send(..).await` (async)
//! -> `ctx.tui_tx.try_send(..)` (sync), matching
//! AGS-806/807/808/809/810/811 precedent. /context is best-effort UI —
//! dropping a status event under 16-cap channel backpressure is
//! preferable to stalling the dispatcher.
//!
//! # Aliases
//!
//! Shipped pre-AGS-814: `&["ctx"]` on the `declare_handler!` stub.
//! Stub was a no-op — no user ever benefited from the alias (the
//! legacy match arm only matched the exact literal "/context"). The
//! AGS-814 body-migrate replaces the stub with the real handler and
//! drops the alias to match the shipped match-arm behaviour (which
//! did NOT accept `/ctx`). `aliases()` returns `&[]`. No drift to
//! reconcile at the user-facing surface.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;

/// Owned snapshot of every value the /context body reads from shared
/// state. Built at the dispatch site (where `.await` is allowed) and
/// threaded through [`CommandContext`] so the sync handler can consume
/// without holding any async-mutex guard on `session_stats`.
///
/// Every field is an owned scalar — no `Arc`, no `Mutex`, no borrows.
/// Pre-capturing the three `session_stats` counters + the two
/// `SlashCommandContext` `usize` fields inside the builder means the
/// handler is zero-`.await` and pays zero additional lock traffic at
/// dispatch time.
#[derive(Debug, Clone)]
pub(crate) struct ContextSnapshot {
    /// Cumulative input tokens observed across every turn in this
    /// session, copied from `SessionStats::input_tokens`.
    pub(crate) input_tokens: u64,
    /// Cumulative output tokens observed across every turn in this
    /// session, copied from `SessionStats::output_tokens`.
    pub(crate) output_tokens: u64,
    /// Conversation turn counter, copied from
    /// `SessionStats::turn_count`.
    pub(crate) turn_count: u64,
    /// System-prompt character size (pre-computed at session init),
    /// copied from `SlashCommandContext::system_prompt_chars` (Copy
    /// usize — no lock required).
    pub(crate) system_prompt_chars: usize,
    /// Tool definitions character size (pre-computed at session init),
    /// copied from `SlashCommandContext::tool_defs_chars` (Copy usize —
    /// no lock required).
    pub(crate) tool_defs_chars: usize,
}

/// Build a [`ContextSnapshot`] by awaiting a single
/// `session_stats.lock()` in the SAME order as the shipped `/context`
/// body at `src/command/slash.rs:267-331`.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/context`. All other commands leave
/// `context_snapshot = None` to avoid unnecessary lock traffic on
/// `session_stats`.
pub(crate) async fn build_context_snapshot(
    slash_ctx: &SlashCommandContext,
) -> ContextSnapshot {
    // Single `session_stats.lock().await`, matching the shipped
    // one-shot read at slash.rs:269. The guard is released at the end
    // of this function — the handler body reads from owned values only.
    let stats = slash_ctx.session_stats.lock().await;
    ContextSnapshot {
        input_tokens: stats.input_tokens,
        output_tokens: stats.output_tokens,
        turn_count: stats.turn_count,
        // `system_prompt_chars` and `tool_defs_chars` are `Copy`
        // usize fields on SlashCommandContext (no lock needed), but
        // we still capture them in the snapshot so the handler sees
        // a single consistent view.
        system_prompt_chars: slash_ctx.system_prompt_chars,
        tool_defs_chars: slash_ctx.tool_defs_chars,
    }
}

/// Zero-sized handler registered as the primary `/context` command.
///
/// Aliases: none. The shipped `declare_handler!` stub declared
/// `&["ctx"]`, but the stub was a no-op and the legacy match arm in
/// `slash.rs` only matched the exact `/context` literal — so `/ctx`
/// never actually worked for users. Dropping the alias aligns the
/// real handler with shipped user-visible behaviour (see module
/// rustdoc "Aliases" section).
pub(crate) struct ContextHandler;

impl CommandHandler for ContextHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // Defensive: build_command_context is responsible for
        // populating context_snapshot when the primary resolves to
        // /context. A None here indicates a wiring regression — surface
        // it as an anyhow::Error so the bug is loud rather than silent.
        // Mirrors AGS-807/808/809/811 defensive pattern.
        let snap = ctx.context_snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "ContextHandler invoked without context_snapshot \
                 populated — build_command_context bug"
            )
        })?;

        // Estimate token counts from character sizes (~4 chars per
        // token) — byte-for-byte from shipped slash.rs:273-275.
        let sys_prompt_tokens = snap.system_prompt_chars as f64 / 4.0;
        let tool_def_tokens = snap.tool_defs_chars as f64 / 4.0;

        // Conversation tokens: input tokens minus the fixed overhead
        // (system prompt + tools are sent every turn). Preserves the
        // shipped `max(input, overhead) - overhead` clamp so negative
        // conversation counts never surface. slash.rs:280-285.
        let fixed_overhead = sys_prompt_tokens + tool_def_tokens;
        let conversation_tokens = if snap.input_tokens > 0 {
            (snap.input_tokens as f64).max(fixed_overhead) - fixed_overhead
        } else {
            0.0
        };

        // Total estimated context = fixed overhead + conversation.
        let total_context = fixed_overhead + conversation_tokens;

        let context_limit = 200_000.0_f64;
        let pct = (total_context / context_limit * 100.0).min(100.0);
        let bar_width = 40usize;
        let filled = (pct / 100.0 * bar_width as f64) as usize;
        let bar: String = format!(
            "[{}{}] {pct:.1}%",
            "#".repeat(filled),
            "-".repeat(bar_width.saturating_sub(filled))
        );

        // Format a token count nicely (e.g. 3.2k or 312).
        let fmt_tok = |t: f64| -> String {
            if t >= 1000.0 {
                format!("{:.1}k", t / 1000.0)
            } else {
                format!("{:.0}", t)
            }
        };

        let input_k = snap.input_tokens as f64 / 1000.0;
        let output_k = snap.output_tokens as f64 / 1000.0;

        let msg = format!(
            "\nContext window usage:\n\
             {bar}\n\
             \n\
             System prompt:    ~{sys} tokens\n\
             Tool definitions: ~{tools} tokens\n\
             Conversation:     ~{conv} tokens\n\
             Total context:    ~{total} / {limit}k tokens\n\
             \n\
             API usage this session:\n\
             Input:  {input_k:.1}k tokens\n\
             Output: {output_k:.1}k tokens\n\
             Turns:  {turns}\n",
            sys = fmt_tok(sys_prompt_tokens),
            tools = fmt_tok(tool_def_tokens),
            conv = fmt_tok(conversation_tokens),
            total = fmt_tok(total_context),
            limit = context_limit as u64 / 1000,
            turns = snap.turn_count,
        );

        // try_send vs send().await: handler is sync. /context is
        // best-effort UI so dropping under channel backpressure (16-cap)
        // is preferable to stalling the dispatcher. Mirrors
        // AGS-806..811 emission primitive.
        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(msg));
        Ok(())
    }

    fn description(&self) -> &str {
        "Show current context window usage"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Shipped stub had &["ctx"] but the legacy match arm only
        // matched "/context" literally — the alias was cosmetic. The
        // real handler drops it to align with actual user-visible
        // behaviour.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-814: tests for /context slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel and the
    /// supplied optional context snapshot. Tests exercising the
    /// defensive None branch pass `None`; tests exercising the happy
    /// path pass `Some(ContextSnapshot { .. })`.
    fn make_ctx(
        snapshot: Option<ContextSnapshot>,
    ) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = mpsc::channel::<TuiEvent>(16);
        (
            CommandContext {
                tui_tx: tx,
                status_snapshot: None,
                model_snapshot: None,
                cost_snapshot: None,
                mcp_snapshot: None,
                context_snapshot: snapshot,
                // TASK-AGS-815: /context tests never exercise /fork paths — None.
                session_id: None,
                // TASK-AGS-817: /context tests never exercise /memory paths — None.
                memory: None,
                // TASK-AGS-POST-6-BODIES-B01-FAST: /context tests never exercise /fast paths — None.
                fast_mode_shared: None,
                // TASK-AGS-POST-6-BODIES-B02-THINKING: /context tests never exercise /thinking paths — None.
                show_thinking: None,
                // TASK-AGS-POST-6-BODIES-B04-DIFF: /context tests never exercise /diff paths — None.
                working_dir: None,
                pending_effect: None,
            },
            rx,
        )
    }

    #[test]
    fn context_handler_description_matches() {
        let h = ContextHandler;
        let desc = h.description().to_lowercase();
        assert!(
            desc.contains("context")
                || desc.contains("window")
                || desc.contains("usage"),
            "ContextHandler description should reference \
             context/window/usage, got: {}",
            h.description()
        );
    }

    #[test]
    fn context_handler_aliases_are_empty() {
        let h = ContextHandler;
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "ContextHandler must register NO aliases — shipped stub's \
             `ctx` alias was cosmetic (legacy match arm only matched \
             `/context` literally). See module rustdoc."
        );
    }

    #[test]
    fn context_handler_execute_with_snapshot_emits_text_delta() {
        let snap = ContextSnapshot {
            input_tokens: 1_000,
            output_tokens: 500,
            turn_count: 3,
            system_prompt_chars: 4_000,
            tool_defs_chars: 2_000,
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));
        ContextHandler
            .execute(&mut ctx, &[])
            .expect("ContextHandler::execute must return Ok with snapshot");

        let ev = rx.try_recv().expect("must emit a TuiEvent");
        match ev {
            TuiEvent::TextDelta(s) => {
                // Byte-for-byte anchors from the shipped format string.
                assert!(
                    s.contains("Context window usage"),
                    "text must include header; got: {s}"
                );
                assert!(
                    s.contains("System prompt:"),
                    "text must include system prompt line; got: {s}"
                );
                assert!(
                    s.contains("Tool definitions:"),
                    "text must include tool defs line; got: {s}"
                );
                assert!(
                    s.contains("Total context:"),
                    "text must include total line; got: {s}"
                );
                assert!(
                    s.contains("Turns:"),
                    "text must include turn count; got: {s}"
                );
                // Turn count rendered as raw integer.
                assert!(
                    s.contains("Turns:  3"),
                    "turn count 3 must surface verbatim; got: {s}"
                );
            }
            other => panic!("expected TuiEvent::TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn context_handler_execute_without_snapshot_returns_err() {
        let (mut ctx, _rx) = make_ctx(None);
        let result = ContextHandler.execute(&mut ctx, &[]);
        assert!(
            result.is_err(),
            "ContextHandler::execute must return Err when \
             context_snapshot is None (defensive: builder bug \
             should surface loudly)"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("context_snapshot")
                || err_msg.contains("build_command_context"),
            "error must describe the missing snapshot, got: {err_msg}"
        );
    }

    #[test]
    fn context_snapshot_round_trip_via_clone() {
        // Sanity: ContextSnapshot derives Debug + Clone and cloning
        // preserves every field. Required because the type is inserted
        // into Option<ContextSnapshot> in CommandContext and read back
        // by the handler.
        let snap = ContextSnapshot {
            input_tokens: 100,
            output_tokens: 50,
            turn_count: 1,
            system_prompt_chars: 100,
            tool_defs_chars: 50,
        };
        let cloned = snap.clone();
        assert_eq!(cloned.input_tokens, 100);
        assert_eq!(cloned.output_tokens, 50);
        assert_eq!(cloned.turn_count, 1);
        assert_eq!(cloned.system_prompt_chars, 100);
        assert_eq!(cloned.tool_defs_chars, 50);
        // Debug impl must not panic.
        let _ = format!("{snap:?}");
    }
}
