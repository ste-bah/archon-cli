//! TASK-AGS-POST-6-BODIES-B16-USAGE: /usage slash-command handler
//! (body-migrate, SNAPSHOT pattern — READ-only).
//!
//! Reference: shipped legacy match arm at
//! `src/command/slash.rs:315-336`. Source: shipped `declare_handler!(
//! UsageHandler, "Show aggregate API usage for the session")` stub at
//! `src/command/registry.rs:1166` (no aliases), registered as primary
//! at `src/command/registry.rs:1278`.
//!
//! # R1 PATTERN-CONFIRM (SNAPSHOT chosen)
//!
//! The shipped legacy body at slash.rs:315-336 acquires a single
//! `tokio::sync::Mutex` guard on `slash_ctx.session_stats`, derives
//! every output line from that guard (input_tokens, output_tokens,
//! turn_count, input_cost, output_cost, total_cost,
//! cache_stats.format_for_cost()), and emits a single
//! `TuiEvent::TextDelta`. Because `CommandHandler::execute` is SYNC
//! (Q1=A invariant), the `.lock().await` is not legal inside `execute`.
//! Solution (same snapshot pattern as AGS-807/808/809/811/814, B08/B11/
//! B12/B14/B15): the builder at `src/command/context.rs` acquires the
//! lock BEFORE calling `Dispatcher::dispatch`, pre-computes every
//! derived value inside the same guard scope, and threads the owned
//! values through [`CommandContext::usage_snapshot`] so the sync handler
//! consumes without holding any lock or borrow.
//!
//! /usage is READ-ONLY — there is no `CommandEffect` variant for this
//! ticket. The `/usage` command never mutates shared state.
//!
//! # R2 PRIMARY-ALREADY-REGISTERED
//!
//! `usage` is already a primary in the default registry via the
//! `declare_handler!(UsageHandler, "Show aggregate API usage for the
//! session")` stub at registry.rs:1166 (no aliases). This ticket is a
//! body-migrate, NOT a gap-fix: primary count is UNCHANGED. The stub is
//! REMOVED in favour of the real type defined in this file, imported at
//! the top of registry.rs, and kept at the existing
//! `insert_primary("usage", Arc::new(UsageHandler::new()))` site.
//!
//! # R3 ALIASES (zero — preserved from shipped)
//!
//! The shipped stub used the two-arg `declare_handler!` form (no
//! aliases slice). Zero aliases preserved. Pinned by test
//! `usage_handler_aliases_are_empty`. Note: `/cost` (AGS-809) wanted
//! `usage` as one of its own aliases but could not register it
//! because `usage` is a shipped primary (this handler). The existence
//! of THIS primary is the reason /cost's alias set is `[billing]` only.
//!
//! # R4 ARG SEMANTICS
//!
//! The shipped arm matched `/usage` literally — no args were consumed.
//! Post-migration, the handler's `args: &[String]` is IGNORED in every
//! branch. Any trailing tokens after `/usage` simply route here and are
//! silently discarded — byte-identical to shipped behaviour.
//!
//! # R5 FORMAT STRING (byte-identity)
//!
//! The shipped format string uses `.4` precision (4 decimals — NOT the
//! `.2` used by /cost) and aligned labels:
//!   - "Turns:         "
//!   - "Input tokens:  "
//!   - "Output tokens: "
//!   - "Total cost:    "
//! The cache_stats_line is inlined verbatim between Output and Total.
//! There is NO warn_threshold, NO hard_limit — those are /cost-only
//! concerns. /usage is the more-detail-than-/cost command per the
//! shipped source comment: "Same as /cost but with more detail".
//!
//! # R6 EMISSION ORDER (unchanged vs shipped)
//!
//! Shipped order:
//!
//! ```ignore
//! // at the tail of the "/usage" => arm in slash.rs:315-336:
//! let _ = tui_tx.send(TuiEvent::TextDelta(msg)).await;
//! ```
//!
//! Post-migration uses `try_send` instead of `.send(..).await`. Every
//! production deployment has a drained mpsc receiver (TUI event loop)
//! so the fast path is identical; `try_send` never blocks and returns
//! `Err` only if the channel is full or closed, which is never true at
//! the /usage dispatch site. Mirrors every prior B-series SNAPSHOT
//! migration.
//!
//! # R7 TEMPORARY DOUBLE-FIRE NOTE (Gates 1-4 scope)
//!
//! For Gates 1-4 of this ticket the legacy match arm at
//! `src/command/slash.rs:315-336` is LEFT INTACT. Because
//! `dispatcher.dispatch` fires the handler BEFORE the recognized-
//! command short-circuit allows fall-through into the match, `/usage`
//! will fire `UsageHandler` AND the legacy arm on every input. Gate 5
//! (live-smoke + legacy-arm deletion) removes the double fire in
//! production. Mirrors every prior B-series migration.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

/// Owned snapshot of every value the /usage body reads from shared
/// state. Built at the dispatch site (where `.await` is allowed) and
/// threaded through [`CommandContext`] so the sync handler can consume
/// without holding locks.
///
/// All fields are plain owned types — no `Arc`, no `Mutex`, no borrows.
/// Every derived value (input_cost, output_cost, total_cost,
/// cache_stats_line) is PRE-COMPUTED inside the builder so the handler
/// never needs access to `CacheStats` (which lives inside the
/// `session_stats` mutex).
///
/// Unlike [`crate::command::cost::CostSnapshot`], this snapshot has
/// NO warn_threshold and NO hard_label — /usage intentionally omits
/// those fields per the shipped format at slash.rs:315-336. /usage
/// also carries a `turn_count` field not present on CostSnapshot.
#[derive(Debug, Clone)]
pub(crate) struct UsageSnapshot {
    /// Cumulative input tokens for the current session. Read from
    /// `SessionStats::input_tokens` inside the mutex guard.
    pub(crate) input_tokens: u64,
    /// Cumulative output tokens for the current session.
    pub(crate) output_tokens: u64,
    /// Cumulative turn count for the current session. Read from
    /// `SessionStats::turn_count` inside the mutex guard. This field
    /// is /usage-specific — /cost does not surface it.
    pub(crate) turn_count: u64,
    /// Pre-computed input cost = `input_tokens * 3.0 / 1_000_000.0`.
    /// Matches the shipped per-million rate at slash.rs:318.
    pub(crate) input_cost: f64,
    /// Pre-computed output cost = `output_tokens * 15.0 / 1_000_000.0`.
    /// Matches the shipped per-million rate at slash.rs:319.
    pub(crate) output_cost: f64,
    /// `input_cost + output_cost` — the "Total cost" tail line.
    pub(crate) total_cost: f64,
    /// Pre-computed via `stats.cache_stats.format_for_cost()` inside
    /// the builder (REQUIRED: `CacheStats` lives inside the
    /// `session_stats` mutex, so the handler cannot format it on its
    /// own without re-acquiring the guard). Three lines joined with
    /// `\n` — inlined as-is between "Output tokens" and "Total cost"
    /// in the final output.
    pub(crate) cache_stats_line: String,
}

/// Build a [`UsageSnapshot`] by awaiting the `session_stats` lock in the
/// SAME order as the shipped `/usage` body at
/// `src/command/slash.rs:315-336` and pre-computing every derived value
/// inside the guard scope.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/usage`. All other commands leave
/// `usage_snapshot = None` to avoid unnecessary lock traffic.
///
/// Rates are hardcoded (`$3 / Mtok input`, `$15 / Mtok output`) — same
/// drift-reconcile decision as the shipped body and the AGS-809 /cost
/// builder. The spec variant with `provider_registry.price_table()` +
/// multi-model breakdown is SCOPE-HELD per orchestrator "shipped wins"
/// rule.
pub(crate) async fn build_usage_snapshot(
    slash_ctx: &SlashCommandContext,
) -> UsageSnapshot {
    // Single mutex acquisition. Every derived value is computed INSIDE
    // this scope so the guard drops before the builder returns.
    let stats = slash_ctx.session_stats.lock().await;

    let input_tokens = stats.input_tokens;
    let output_tokens = stats.output_tokens;
    let turn_count = stats.turn_count;
    let input_cost = input_tokens as f64 * 3.0 / 1_000_000.0;
    let output_cost = output_tokens as f64 * 15.0 / 1_000_000.0;
    let total_cost = input_cost + output_cost;

    // REQUIRED: format_for_cost() must be called while we still hold
    // the guard because CacheStats lives inside SessionStats. The
    // resulting String is owned and moves freely after the guard drop.
    let cache_stats_line = stats.cache_stats.format_for_cost();

    UsageSnapshot {
        input_tokens,
        output_tokens,
        turn_count,
        input_cost,
        output_cost,
        total_cost,
        cache_stats_line,
    }
    // Guard drops here — lock released before return.
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// Real `/usage` handler — consumes a pre-built [`UsageSnapshot`]
/// from [`CommandContext`] and emits the composed text as a single
/// `TuiEvent::TextDelta` via `try_send`.
///
/// # Branch matrix
///
/// * `usage_snapshot == None` → `anyhow::Err` (wiring regression —
///   `build_command_context` bypassed or alias map drifted). Mirrors
///   B15 `DoctorHandler` defensive stance.
/// * `usage_snapshot == Some(snap)` → single
///   `TuiEvent::TextDelta(format!(...))` via `try_send` using the
///   shipped byte-identical format string with `.4` precision and
///   aligned labels.
pub(crate) struct UsageHandler;

impl UsageHandler {
    /// Default production constructor — zero-sized struct, no state.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for UsageHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for UsageHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // R4: args are IGNORED. The shipped legacy arm at
        // slash.rs:315-336 matched `/usage` literally with no
        // strip_prefix — trailing tokens were silently discarded.
        // Preserved here via the `_args` parameter rename.

        // Defensive: build_command_context is responsible for populating
        // usage_snapshot when the primary resolves to /usage. A None
        // here indicates a wiring regression (e.g. the builder was
        // bypassed or the alias map drifted), not a user-facing error —
        // but we surface it as an anyhow::Error so the bug is loud
        // rather than silent. Mirrors AGS-807/808/809 + B15 defensive
        // pattern.
        let snap = ctx.usage_snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "UsageHandler invoked without usage_snapshot populated \
                 — build_command_context bug"
            )
        })?;

        // Byte-for-byte faithful to shipped body at slash.rs:315-336.
        // Field order, label alignment spaces, `.4` precision, and
        // trailing newlines preserved verbatim: leading `\n`,
        // "Usage summary:", "Turns:         ", "Input tokens:  ",
        // "Output tokens: ", cache_stats_line inlined, "Total cost:    ",
        // trailing `\n`.
        let msg = format!(
            "\nUsage summary:\n\
             Turns:         {turns}\n\
             Input tokens:  {inp} (${input_cost:.4})\n\
             Output tokens: {out} (${output_cost:.4})\n\
             {cache_line}\n\
             Total cost:    ${total:.4}\n",
            turns = snap.turn_count,
            inp = snap.input_tokens,
            input_cost = snap.input_cost,
            out = snap.output_tokens,
            output_cost = snap.output_cost,
            cache_line = snap.cache_stats_line,
            total = snap.total_cost,
        );

        // Sync try_send analogous to /cost, /status, /model, /doctor
        // precedent. Dropping a /usage line under backpressure is
        // preferable to stalling the input pipeline.
        ctx.emit(TuiEvent::TextDelta(msg));
        Ok(())
    }

    fn description(&self) -> &str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:1166 (shipped-wins drift-reconcile).
        "Show aggregate API usage for the session"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R3: zero aliases shipped → zero aliases preserved. Pinned by
        // test `usage_handler_aliases_are_empty`. NOTE: /cost (AGS-809)
        // wanted `usage` as one of its aliases but could not register
        // it because /usage is a shipped primary (this handler).
        &[]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    #[test]
    fn usage_handler_description_byte_identical_to_shipped() {
        let h = UsageHandler::new();
        assert_eq!(
            h.description(),
            "Show aggregate API usage for the session",
            "UsageHandler description must match the shipped \
             declare_handler! stub at registry.rs:1166 byte-for-byte \
             (shipped-wins drift-reconcile)"
        );
    }

    #[test]
    fn usage_handler_aliases_are_empty() {
        let h = UsageHandler::new();
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "UsageHandler aliases must be empty to match the shipped \
             declare_handler! stub (two-arg form, no aliases slice)"
        );
    }

    #[test]
    fn usage_handler_execute_without_snapshot_returns_err() {
        let (mut ctx, _rx) = make_usage_ctx(None);
        let h = UsageHandler::new();
        let result = h.execute(&mut ctx, &[]);
        assert!(
            result.is_err(),
            "UsageHandler::execute must return Err when usage_snapshot \
             is None (defensive: builder bug should surface loudly)"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("usage_snapshot")
                || err_msg.contains("build_command_context"),
            "error must describe the missing snapshot, got: {err_msg}"
        );
    }

    #[test]
    fn usage_handler_execute_with_snapshot_emits_text_delta_byte_identical() {
        let (mut ctx, mut rx) = make_usage_ctx(Some(fixture_usage_snapshot()));
        let h = UsageHandler::new();
        h.execute(&mut ctx, &[])
            .expect("UsageHandler::execute must return Ok with snapshot populated");

        let ev = rx.try_recv().expect("must emit a TuiEvent");
        match ev {
            TuiEvent::TextDelta(msg) => {
                let expected = "\nUsage summary:\n\
                                Turns:         3\n\
                                Input tokens:  1000000 ($3.0000)\n\
                                Output tokens: 500000 ($7.5000)\n\
                                Cache hit rate: 0.0% (0 reads / 0 total)\n\
                                Cache creation: 0 tokens\n\
                                Estimated savings: 0 token-equivalents\n\
                                Total cost:    $10.5000\n";
                assert_eq!(
                    msg, expected,
                    "TextDelta must match shipped /usage format \
                     byte-for-byte (slash.rs:315-336)"
                );
            }
            other => panic!("expected TuiEvent::TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn usage_snapshot_round_trip_via_clone() {
        // Cheap sanity check that UsageSnapshot derives Debug + Clone and
        // that cloning preserves every field. Required because the type
        // is inserted into Option<UsageSnapshot> in CommandContext and
        // read back by the handler (no Copy on String).
        let snap = fixture_usage_snapshot();
        let cloned = snap.clone();
        assert_eq!(snap.input_tokens, cloned.input_tokens);
        assert_eq!(snap.output_tokens, cloned.output_tokens);
        assert_eq!(snap.turn_count, cloned.turn_count);
        assert!((snap.input_cost - cloned.input_cost).abs() < f64::EPSILON);
        assert!((snap.output_cost - cloned.output_cost).abs() < f64::EPSILON);
        assert!((snap.total_cost - cloned.total_cost).abs() < f64::EPSILON);
        assert_eq!(snap.cache_stats_line, cloned.cache_stats_line);
        // Debug impl must not panic.
        let _ = format!("{snap:?}");
    }

    // ---- Gate 5: dispatcher-integration end-to-end --------------------
    //
    // Route `/usage` through the real Dispatcher + Registry + handler
    // stack to pin post-arm-delete wiring. Builds a NARROW
    // `RegistryBuilder` (not `default_registry`) to keep the test scope
    // tight — only `/usage` is registered, so any routing regression
    // surfaces immediately rather than getting masked by unrelated
    // handlers. Mirrors B15 DoctorHandler dispatcher tests.

    #[test]
    fn dispatcher_routes_slash_usage_with_snapshot_emits_textdelta() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::RegistryBuilder;
        use std::sync::Arc;

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("usage", Arc::new(UsageHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, mut rx) = make_usage_ctx(Some(fixture_usage_snapshot()));

        let res = dispatcher.dispatch(&mut ctx, "/usage");
        assert!(
            res.is_ok(),
            "dispatcher.dispatch must return Ok when snapshot is \
             populated, got: {res:?}"
        );
        assert!(
            ctx.pending_effect.is_none(),
            "SNAPSHOT pattern must never produce a pending_effect, got: \
             {:?}",
            ctx.pending_effect
        );

        let events = drain_tui_events(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "exactly one event must be emitted through the dispatcher, \
             got: {events:?}"
        );
        let expected = "\nUsage summary:\n\
                        Turns:         3\n\
                        Input tokens:  1000000 ($3.0000)\n\
                        Output tokens: 500000 ($7.5000)\n\
                        Cache hit rate: 0.0% (0 reads / 0 total)\n\
                        Cache creation: 0 tokens\n\
                        Estimated savings: 0 token-equivalents\n\
                        Total cost:    $10.5000\n";
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, expected,
                    "TextDelta must carry the shipped /usage format \
                     byte-for-byte through Dispatcher -> Registry -> \
                     UsageHandler::execute"
                );
            }
            other => panic!(
                "dispatcher must route /usage to a TextDelta emission, \
                 got: {other:?}"
            ),
        }
        // No Error event emitted on the happy path.
        assert!(
            events.iter().all(|ev| !matches!(ev, TuiEvent::Error(_))),
            "happy path must not emit any TuiEvent::Error, got: {events:?}"
        );
    }

    #[test]
    fn dispatcher_routes_slash_usage_without_snapshot_returns_err() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::RegistryBuilder;
        use std::sync::Arc;

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("usage", Arc::new(UsageHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, mut rx) = make_usage_ctx(None);

        let res = dispatcher.dispatch(&mut ctx, "/usage");
        assert!(
            res.is_err(),
            "dispatcher.dispatch must propagate UsageHandler's Err when \
             usage_snapshot is None (builder contract violation), got: \
             {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.to_lowercase().contains("usage_snapshot"),
            "dispatcher-propagated Err must mention 'usage_snapshot', \
             got: {msg}"
        );

        // No TextDelta emitted on the Err path — the handler short-
        // circuits before try_send.
        let events = drain_tui_events(&mut rx);
        assert!(
            events.is_empty(),
            "Err path must not emit any TuiEvent, got: {events:?}"
        );
    }
}
