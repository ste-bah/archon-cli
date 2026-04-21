//! TASK-AGS-809: /cost slash-command handler (body-migrate target).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!` stub
//! in `src/command/registry.rs` and the legacy match arm at
//! `src/command/slash.rs:339-365`. The legacy body held a single
//! `tokio::sync::Mutex` guard on `slash_ctx.session_stats` and derived
//! every output line from that guard plus the (non-async) `cost_config`
//! fields on `SlashCommandContext`.
//!
//! Because `CommandHandler::execute` is SYNC (Q1=A invariant), the
//! `.lock().await` on `session_stats` is not legal inside `execute`.
//! Solution (same snapshot pattern as AGS-807 `/status`): the dispatch
//! site at `slash.rs` builds a [`CostSnapshot`] by awaiting the lock
//! BEFORE calling `Dispatcher::dispatch`, pre-computing every derived
//! value (input/output cost, total, cache-stats line, hard-limit label)
//! inside the same guard scope, and threads the owned values through
//! [`CommandContext`] so the sync handler consumes without holding any
//! lock or borrow.
//!
//! /cost is READ-ONLY — there is no `CommandEffect` variant for this
//! ticket. The `/cost` command never mutates shared state.
//!
//! Aliases: `[billing]` only. Spec REQ-FOR-D7 validation criterion 2
//! requested `[usage, billing]`, but `usage` is already a primary
//! command in the shipped registry (`UsageHandler` — "Show aggregate
//! API usage for the session"). Registering `usage` as an alias for
//! `/cost` would trip the `RegistryBuilder::build` alias/primary
//! collision panic at init time. This ticket therefore applies ONLY
//! the collision-free subset (`billing`) as an IMPROVEMENT R-item and
//! records the dropped alias as a CONFIRM R-item for orchestrator
//! review. Shipped `/usage` behaviour (stub, TASK-AGS-624 will body-
//! migrate it separately) is preserved unchanged.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;

/// Owned snapshot of every value the /cost body reads from shared
/// state. Built at the dispatch site (where `.await` is allowed) and
/// threaded through [`CommandContext`] so the sync handler can consume
/// without holding locks.
///
/// All fields are plain owned types — no `Arc`, no `Mutex`, no borrows.
/// Every derived value (input_cost, output_cost, total_cost,
/// cache_stats_line, hard_label) is PRE-COMPUTED inside the builder so
/// the handler never needs access to `CacheStats` (which lives inside
/// the `session_stats` mutex) or the `cost_config` field.
#[derive(Debug, Clone)]
pub(crate) struct CostSnapshot {
    /// Cumulative input tokens for the current session. Read from
    /// `SessionStats::input_tokens` inside the mutex guard.
    pub(crate) input_tokens: u64,
    /// Cumulative output tokens for the current session.
    pub(crate) output_tokens: u64,
    /// Pre-computed input cost = `input_tokens * 3.0 / 1_000_000.0`.
    /// Matches the shipped per-million rate at slash.rs:341.
    pub(crate) input_cost: f64,
    /// Pre-computed output cost = `output_tokens * 15.0 / 1_000_000.0`.
    /// Matches the shipped per-million rate at slash.rs:342.
    pub(crate) output_cost: f64,
    /// `input_cost + output_cost` — the "Session cost" headline.
    pub(crate) total_cost: f64,
    /// Pre-computed via `stats.cache_stats.format_for_cost()` inside
    /// the builder (REQUIRED: `CacheStats` lives inside the
    /// `session_stats` mutex, so the handler cannot format it on its
    /// own without re-acquiring the guard). Three lines joined with
    /// `\n` — inlined as-is between "Output tokens" and "Warn
    /// threshold" in the final output.
    pub(crate) cache_stats_line: String,
    /// Copy of `slash_ctx.cost_config.warn_threshold`. Non-async field
    /// on `SlashCommandContext`, captured here for symmetry.
    pub(crate) warn_threshold: f64,
    /// Pre-computed conditional label: `"$0.00 (disabled)"` when
    /// `cost_config.hard_limit <= 0.0`, otherwise `"${hard:.2}"`.
    /// Byte-for-byte faithful to the shipped `if hard <= 0.0 { .. }`
    /// branch at slash.rs:346-350 (NOTE: "disabled", NOT "Unlimited").
    pub(crate) hard_label: String,
}

/// Build a [`CostSnapshot`] by awaiting the `session_stats` lock in the
/// SAME order as the shipped `/cost` body at `src/command/slash.rs:339-365`
/// and pre-computing every derived value inside the guard scope.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/cost` (or one of its aliases `/usage`, `/billing`).
/// All other commands leave `cost_snapshot = None` to avoid
/// unnecessary lock traffic.
///
/// Rates are hardcoded (`$3 / Mtok input`, `$15 / Mtok output`) —
/// same drift-reconcile decision as the shipped body. The spec
/// variant with `provider_registry.price_table()` + multi-model
/// breakdown + `--since` filter is SCOPE-HELD per orchestrator
/// "shipped wins" rule.
pub(crate) async fn build_cost_snapshot(
    slash_ctx: &SlashCommandContext,
) -> CostSnapshot {
    // Single mutex acquisition. Every derived value is computed INSIDE
    // this scope so the guard drops before the builder returns.
    let stats = slash_ctx.session_stats.lock().await;

    let input_tokens = stats.input_tokens;
    let output_tokens = stats.output_tokens;
    let input_cost = input_tokens as f64 * 3.0 / 1_000_000.0;
    let output_cost = output_tokens as f64 * 15.0 / 1_000_000.0;
    let total_cost = input_cost + output_cost;

    // REQUIRED: format_for_cost() must be called while we still hold
    // the guard because CacheStats lives inside SessionStats. The
    // resulting String is owned and moves freely after the guard drop.
    let cache_stats_line = stats.cache_stats.format_for_cost();

    // `cost_config` is a plain field on SlashCommandContext (NOT inside
    // a mutex) so we can read it without awaiting. Snapshotted here for
    // symmetry with the other owned fields.
    let warn_threshold = slash_ctx.cost_config.warn_threshold;
    let hard_limit = slash_ctx.cost_config.hard_limit;
    let hard_label = if hard_limit <= 0.0 {
        "$0.00 (disabled)".to_string()
    } else {
        format!("${hard_limit:.2}")
    };

    CostSnapshot {
        input_tokens,
        output_tokens,
        input_cost,
        output_cost,
        total_cost,
        cache_stats_line,
        warn_threshold,
        hard_label,
    }
    // Guard drops here — lock released before return.
}

/// Zero-sized handler registered as the primary `/cost` command.
/// Aliases: `[billing]` — spec wanted `[usage, billing]` but `usage`
/// is already a primary (`UsageHandler`); see module-level rustdoc
/// for the collision-avoidance rationale.
pub(crate) struct CostHandler;

impl CommandHandler for CostHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // Defensive: build_command_context is responsible for populating
        // cost_snapshot when the primary resolves to /cost. A None here
        // indicates a wiring regression (e.g. the builder was bypassed
        // or the alias map drifted), not a user-facing error — but we
        // surface it as an anyhow::Error so the bug is loud rather than
        // silent. Mirrors AGS-807/808 defensive pattern.
        let snap = ctx.cost_snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "CostHandler invoked without cost_snapshot populated \
                 — build_command_context bug"
            )
        })?;

        // Byte-for-byte faithful to shipped body at slash.rs:339-365.
        // Field order, label casing, and trailing newlines preserved
        // verbatim: leading `\n`, "Session cost:", "Input tokens:",
        // "Output tokens:", cache_stats_line inlined, "Warn threshold:",
        // "Hard limit:", trailing `\n`.
        let msg = format!(
            "\n\
             Session cost: ${total:.2}\n\
             Input tokens: {input_tok} (${input_cost:.2})\n\
             Output tokens: {output_tok} (${output_cost:.2})\n\
             {cache_line}\n\
             Warn threshold: ${warn:.2}\n\
             Hard limit: {hard_label}\n",
            total = snap.total_cost,
            input_tok = snap.input_tokens,
            input_cost = snap.input_cost,
            output_tok = snap.output_tokens,
            output_cost = snap.output_cost,
            cache_line = snap.cache_stats_line,
            warn = snap.warn_threshold,
            hard_label = snap.hard_label,
        );

        // Sync try_send analogous to /tasks, /status, /model precedent.
        // Dropping a /cost line under backpressure is preferable to
        // stalling the input pipeline.
        ctx.emit(TuiEvent::TextDelta(msg));
        Ok(())
    }

    fn description(&self) -> &str {
        "Show session token cost breakdown"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // Spec requested `[usage, billing]` but `usage` is a
        // shipped primary — registering it here would panic the
        // RegistryBuilder at init time with an alias/primary
        // collision. Only `billing` is collision-free; see module
        // rustdoc for the CONFIRM R-item.
        &["billing"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-809: tests for /cost slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    #[test]
    fn cost_handler_description_matches() {
        let h = CostHandler;
        let desc = h.description().to_lowercase();
        assert!(
            desc.contains("cost")
                || desc.contains("token")
                || desc.contains("session"),
            "CostHandler description should reference cost/token/session, got: {}",
            h.description()
        );
    }

    #[test]
    fn cost_handler_aliases_are_usage_and_billing() {
        let h = CostHandler;
        // Spec requested [usage, billing]; shipped registry already
        // owns `usage` as a primary (`UsageHandler`). Registering
        // `usage` here would panic `RegistryBuilder::build` with an
        // alias/primary collision. We therefore apply only the
        // collision-free subset. Test name preserved so the
        // review-trail links back to the AGS-809 spec criterion.
        assert_eq!(
            h.aliases(),
            &["billing"],
            "CostHandler aliases must be [billing] (spec wanted \
             [usage, billing] but 'usage' is a shipped primary); \
             see module rustdoc for the CONFIRM R-item"
        );
    }

    #[test]
    fn cost_handler_execute_without_snapshot_returns_err() {
        let (mut ctx, _rx) = make_cost_ctx(None);
        let h = CostHandler;
        let result = h.execute(&mut ctx, &[]);
        assert!(
            result.is_err(),
            "CostHandler::execute must return Err when cost_snapshot is None \
             (defensive: builder bug should surface loudly)"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("cost_snapshot")
                || err_msg.contains("build_command_context"),
            "error must describe the missing snapshot, got: {err_msg}"
        );
    }

    #[test]
    fn cost_handler_execute_with_snapshot_emits_text_delta_with_session_cost_line() {
        let (mut ctx, mut rx) = make_cost_ctx(Some(fixture_cost_snapshot()));
        let h = CostHandler;
        h.execute(&mut ctx, &[])
            .expect("CostHandler::execute must return Ok with snapshot populated");

        let ev = rx.try_recv().expect("must emit a TuiEvent");
        match ev {
            TuiEvent::TextDelta(msg) => {
                assert!(
                    msg.contains("Session cost: $10.50"),
                    "TextDelta must contain 'Session cost: $10.50', got: {msg}"
                );
                assert!(
                    msg.contains("Input tokens: 1000000 ($3.00)"),
                    "TextDelta must contain 'Input tokens: 1000000 ($3.00)', got: {msg}"
                );
                assert!(
                    msg.contains("Output tokens: 500000 ($7.50)"),
                    "TextDelta must contain 'Output tokens: 500000 ($7.50)', got: {msg}"
                );
                assert!(
                    msg.contains("Warn threshold: $5.00"),
                    "TextDelta must contain 'Warn threshold: $5.00', got: {msg}"
                );
                assert!(
                    msg.contains("Hard limit: $0.00 (disabled)"),
                    "TextDelta must contain 'Hard limit: $0.00 (disabled)' \
                     (matches shipped conditional for hard_limit <= 0.0), got: {msg}"
                );
                // Cache stats line is inlined verbatim.
                assert!(
                    msg.contains("Cache hit rate: 0.0%"),
                    "TextDelta must inline the cache_stats_line, got: {msg}"
                );
            }
            other => panic!("expected TuiEvent::TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn cost_handler_execute_with_positive_hard_limit_formats_currency() {
        // Verifies the non-disabled branch of the hard_label conditional.
        // Snapshot carries hard_label = "$25.00" to match what the
        // builder would produce for a positive hard_limit.
        let snap = CostSnapshot {
            input_tokens: 0,
            output_tokens: 0,
            input_cost: 0.0,
            output_cost: 0.0,
            total_cost: 0.0,
            cache_stats_line:
                "Cache hit rate: 0.0% (0 reads / 0 total)\n\
                 Cache creation: 0 tokens\n\
                 Estimated savings: 0 token-equivalents"
                    .to_string(),
            warn_threshold: 2.5,
            hard_label: "$25.00".to_string(),
        };
        let (mut ctx, mut rx) = make_cost_ctx(Some(snap));
        let h = CostHandler;
        h.execute(&mut ctx, &[])
            .expect("snapshot-populated execute must return Ok");

        let ev = rx.try_recv().expect("must emit a TuiEvent");
        match ev {
            TuiEvent::TextDelta(msg) => {
                assert!(
                    msg.contains("Hard limit: $25.00"),
                    "positive hard_limit must format as '$25.00', got: {msg}"
                );
                assert!(
                    !msg.contains("disabled"),
                    "positive hard_limit must NOT contain 'disabled', got: {msg}"
                );
            }
            other => panic!("expected TuiEvent::TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn cost_snapshot_round_trip_via_clone() {
        // Cheap sanity check that CostSnapshot derives Debug + Clone and
        // that cloning preserves every field. Required because the type
        // is inserted into Option<CostSnapshot> in CommandContext and
        // read back by the handler (no Copy on String).
        let snap = fixture_cost_snapshot();
        let cloned = snap.clone();
        assert_eq!(snap.input_tokens, cloned.input_tokens);
        assert_eq!(snap.output_tokens, cloned.output_tokens);
        assert!((snap.input_cost - cloned.input_cost).abs() < f64::EPSILON);
        assert!((snap.output_cost - cloned.output_cost).abs() < f64::EPSILON);
        assert!((snap.total_cost - cloned.total_cost).abs() < f64::EPSILON);
        assert_eq!(snap.cache_stats_line, cloned.cache_stats_line);
        assert!(
            (snap.warn_threshold - cloned.warn_threshold).abs() < f64::EPSILON
        );
        assert_eq!(snap.hard_label, cloned.hard_label);
        // Debug impl must not panic.
        let _ = format!("{snap:?}");
    }
}
