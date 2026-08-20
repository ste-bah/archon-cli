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
//! # Output
//!
//! Issue #37 added two lines to the block described below: a `Fixed overhead:`
//! subtotal (system prompt + tool definitions — the part of every request that
//! is resent verbatim and therefore cannot be compacted away) and a
//! `Last request:` line carrying the size of the most recent request body put
//! on the wire. Between them and the existing `Source:` line, the three
//! quantities a TPM stall has to tell apart — window source, fixed overhead,
//! and latest request-body pressure — are now separately visible. The AGS-814
//! migration otherwise preserves the deleted slash.rs:267-331 body byte for
//! byte:
//! - `context_limit` = resolved startup model context window when known.
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

use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;
use archon_tui::app::TuiEvent;

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
    pub(crate) cache_creation_tokens: u64,
    pub(crate) cache_read_tokens: u64,
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
    /// Resolved context window for the active startup model. Zero means unknown.
    pub(crate) context_window: u64,
    /// Resolution source shown to the user (config-override/catalog/provider/etc.).
    pub(crate) context_source: String,
    /// Approximate token size of the most recent request body put on the wire,
    /// copied from `SessionStats::last_request_body_tokens`. Zero before the
    /// first request of the session.
    pub(crate) last_request_body_tokens: u64,
}

/// Build a [`ContextSnapshot`] by awaiting a single
/// `session_stats.lock()` in the SAME order as the shipped `/context`
/// body at `src/command/slash.rs:267-331`.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/context`. All other commands leave
/// `context_snapshot = None` to avoid unnecessary lock traffic on
/// `session_stats`.
pub(crate) async fn build_context_snapshot(slash_ctx: &SlashCommandContext) -> ContextSnapshot {
    // Single `session_stats.lock().await`, matching the shipped
    // one-shot read at slash.rs:269. The guard is released at the end
    // of this function — the handler body reads from owned values only.
    let stats = slash_ctx.session_stats.lock().await;
    ContextSnapshot {
        input_tokens: stats.input_tokens,
        output_tokens: stats.output_tokens,
        cache_creation_tokens: stats.cache_stats.cache_creation_tokens,
        cache_read_tokens: stats.cache_stats.cache_read_tokens,
        turn_count: stats.turn_count,
        // `system_prompt_chars` and `tool_defs_chars` are `Copy`
        // usize fields on SlashCommandContext (no lock needed), but
        // we still capture them in the snapshot so the handler sees
        // a single consistent view.
        system_prompt_chars: slash_ctx.system_prompt_chars,
        tool_defs_chars: slash_ctx.tool_defs_chars,
        context_window: slash_ctx.context_window,
        context_source: slash_ctx.context_source.clone(),
        last_request_body_tokens: stats.last_request_body_tokens,
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

/// One line per message of the current session, for the attribution overlay.
///
/// Empty when there is no session store or the log cannot be read: the ranking
/// still opens, with indices and token counts and no prose. A missing label is
/// a worse overlay; a missing overlay is no answer at all.
fn message_previews_for(ctx: &CommandContext) -> Vec<(usize, String, String)> {
    let (Some(store), Some(session_id)) = (ctx.session_store.as_ref(), ctx.session_id.as_ref())
    else {
        return Vec::new();
    };
    match store.load_messages(session_id) {
        Ok(messages) => crate::command::fork_at::message_previews(&messages),
        Err(error) => {
            tracing::debug!("/context could not read the session log: {error}");
            Vec::new()
        }
    }
}

impl CommandHandler for ContextHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
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

        let context_limit = snap.context_window as f64;
        let pct = if context_limit > 0.0 {
            (total_context / context_limit * 100.0).min(100.0)
        } else {
            0.0
        };
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

        let limit_label = if snap.context_window > 0 {
            format!("{}k", snap.context_window / 1000)
        } else {
            "unknown".to_string()
        };

        // Issue #37: the three quantities a TPM stall needs told apart.
        //
        // A session can sit well under its context-window percentage and still
        // fail on request size, so "47% of the window" alone never explained
        // the failure. `Fixed overhead` names the part of every request that is
        // resent verbatim each turn (and so cannot be compacted away), and
        // `Last request` reports what was actually serialized onto the wire —
        // measured before the send, so a rate-limited request still reports it.
        let last_request_label = if snap.last_request_body_tokens > 0 {
            format!("~{} tokens", fmt_tok(snap.last_request_body_tokens as f64))
        } else {
            "no request sent yet".to_string()
        };
        let msg = format!(
            "\nContext window usage:\n\
             {bar}\n\
             \n\
             System prompt:    ~{sys} tokens\n\
             Tool definitions: ~{tools} tokens\n\
             Fixed overhead:   ~{fixed} tokens (resent every request)\n\
             Conversation:     ~{conv} tokens\n\
             Total context:    ~{total} / {limit} tokens\n\
             Source:           {source}\n\
             Last request:     {last_request}\n\
             \n\
             API usage this session:\n\
             Input:  {input_k:.1}k tokens\n\
             Output: {output_k:.1}k tokens\n\
             Cache:  create {cache_create} / read {cache_read} tokens\n\
             Turns:  {turns}\n",
            sys = fmt_tok(sys_prompt_tokens),
            tools = fmt_tok(tool_def_tokens),
            fixed = fmt_tok(fixed_overhead),
            conv = fmt_tok(conversation_tokens),
            total = fmt_tok(total_context),
            limit = limit_label,
            source = snap.context_source,
            last_request = last_request_label,
            cache_create = snap.cache_creation_tokens,
            cache_read = snap.cache_read_tokens,
            turns = snap.turn_count,
        );

        // try_send vs send().await: handler is sync. /context is
        // best-effort UI so dropping under channel backpressure (16-cap)
        // is preferable to stalling the dispatcher. Mirrors
        // AGS-806..811 emission primitive.
        ctx.emit(TuiEvent::TextDelta(msg));
        // Additive, like every other restored surface (#192): the block above
        // is what a `-p` run keeps, and the overlay is dropped there.
        //
        // The ranking itself is not sent from here. It arrives on
        // `ContextPressureUpdated` because only the agent holds the calibrated
        // token surface; what this side has, and the agent does not, is the
        // message text for those indices. So each half sends what it knows and
        // the event loop joins them.
        if _args.is_empty() {
            ctx.emit(TuiEvent::ShowTokenAttribution(message_previews_for(ctx)));
        }
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
#[path = "context_cmd_tests.rs"]
mod tests;
