//! TASK-AGS-POST-6-BODIES-B11-EFFORT: /effort slash-command handler
//! (body-migrate, HYBRID pattern — SNAPSHOT + EFFECT-SLOT + SIDECAR).
//!
//! Reference: shipped inline match arm at `src/command/slash.rs:92-122`.
//! Source:   shipped `declare_handler!(EffortHandler, "Show or set
//!           reasoning effort (high|medium|low)")` stub at
//!           `src/command/registry.rs:801` (no aliases).
//!
//! # R1 PATTERN-CONFIRM (HYBRID chosen)
//!
//! The shipped body at slash.rs:92-122 performs THREE actions that
//! cannot all run inside a sync `CommandHandler::execute`:
//!
//! 1. **Read** `effort_state.level()` — sync, but `effort_state` is a
//!    `&mut EffortState` local to the session loop (NOT part of
//!    `SlashCommandContext`).
//! 2. **Write** `effort_state.set_level(level)` — sync, local stack var.
//! 3. **Write** `*ctx.effort_level_shared.lock().await = level` — async
//!    on a `tokio::sync::Mutex<EffortLevel>` field of
//!    `SlashCommandContext`.
//!
//! No single existing pattern accommodates all three. HYBRID = SNAPSHOT
//! (AGS-807/808 precedent) + EFFECT-SLOT (AGS-808/B04-DIFF/B10-ADDDIR
//! precedent) + a new SIDECAR slot for the local `EffortState` mutation:
//!
//! * **READ side → SNAPSHOT pattern**. A new
//!   `effort_snapshot: Option<EffortSnapshot>` field on `CommandContext`
//!   is populated by `build_command_context` ONLY when the input
//!   starts with `/effort`. The builder awaits
//!   `slash_ctx.effort_level_shared.lock().await` and stores an owned
//!   `EffortLevel` so the sync handler can render the current-level
//!   line without locking. Verified equivalent to the shipped
//!   `effort_state.level()` read because both fields are mutated in
//!   lockstep (see session.rs init + slash.rs:108-109 paired writes).
//!
//! * **ASYNC WRITE side → EFFECT-SLOT pattern**. A new
//!   `CommandEffect::SetEffortLevelShared(EffortLevel)` variant. The
//!   sync handler stashes the effect via `ctx.pending_effect`.
//!   `apply_effect` in `src/command/context.rs` awaits
//!   `*slash_ctx.effort_level_shared.lock().await = level` at the
//!   dispatch site where `.await` is legal. Mirrors AGS-808
//!   `SetModelOverride` and B10 `AddExtraDir`.
//!
//! * **LOCAL WRITE side → NEW SIDECAR slot**. A new
//!   `pending_effort_set: Option<EffortLevel>` field on
//!   `CommandContext`. The handler stashes BOTH the `CommandEffect`
//!   (for the shared mutex) AND this sidecar (for the local
//!   `EffortState`). The sidecar is drained at the slash.rs dispatch
//!   site AFTER `apply_effect`, where the `&mut EffortState` parameter
//!   is still in scope.
//!
//! # R2 PRIMARY-ALREADY-REGISTERED
//!
//! `effort` is already a primary in the default registry via the
//! `declare_handler!(EffortHandler, "Show or set reasoning effort
//! (high|medium|low)")` stub at registry.rs:801 (no aliases). This
//! ticket is a body-migrate, NOT a gap-fix: primary count is UNCHANGED.
//! The stub is REMOVED in favour of the real type defined in this file,
//! imported into registry.rs at the top via
//! `use crate::command::effort::EffortHandler;`.
//!
//! # R3 NO-ALIASES (shipped-wins drift-reconcile)
//!
//! Shipped `declare_handler!` stub at registry.rs:801 carried no alias
//! slice — equivalent to `&[]`. AGS-817 shipped-wins drift-reconcile
//! rule preserves zero aliases. This handler returns `&[]` from
//! `aliases()` and the test `effort_handler_aliases_are_empty` pins
//! the invariant against silent additions.
//!
//! # R4 ARGS-RECONCILIATION
//!
//! Shipped body uses `s.strip_prefix("/effort").unwrap_or("").trim()`
//! on the raw input string — a single-string substring after the
//! command name. The parser tokenizes on whitespace into
//! `args: &[String]`. For a single-token effort level
//! (`/effort high`), `args.first()` would be byte-equivalent. The
//! handler uses `args.join(" ").trim()` which preserves the shipped
//! substring semantics EXACTLY for any multi-token input and degrades
//! gracefully to the same single-token form for the common case.
//! Empty args (bare `/effort`) and a whitespace-only join both
//! produce the empty string, routing to the help branch identical to
//! the shipped `if level_str.is_empty()` check. Mirrors AGS-819 /theme
//! R4, B09-COLOR R4, and B10-ADDDIR R4.
//!
//! # R5 EMISSION-PRIMITIVE-SWAP (.await -> try_send)
//!
//! Shipped body emitted via `tui_tx.send(..).await` — async, blocking
//! on backpressure if the 16-cap channel is full. The sync
//! `CommandHandler::execute` signature cannot `.await`, so this
//! handler uses `ctx.tui_tx.try_send(..)` (sync, best-effort drop on
//! full). Matches AGS-806..819 emission precedent verbatim. All three
//! shipped format strings are preserved BYTE-FOR-BYTE:
//!
//! 1. `"\nCurrent effort level: {}\nUsage: /effort <high|medium|low>\n"`
//!    (empty-arg branch — `{}` is the snapshot level's `Display`
//!    impl, which yields `"high"` / `"medium"` / `"low"`).
//! 2. `"\nEffort level set to {level}.\n"` (success branch —
//!    `{level}` is the parsed `EffortLevel::Display`).
//! 3. Validation error — pass-through from
//!    `archon_tools::validation::validate_effort_level(level_str)`
//!    `Err(msg)` byte-for-byte (no wrapping, no rewrite).
//!
//! # R6 ORDER-SEMANTICS-SWAP (accepted)
//!
//! Shipped order at slash.rs:108-114:
//!
//! ```ignore
//! effort_state.set_level(level);                           // 1. local
//! *ctx.effort_level_shared.lock().await = level;           // 2. shared
//! let _ = tui_tx.send(TuiEvent::TextDelta(..)).await;      // 3. emit
//! ```
//!
//! Post-migration order:
//!
//! 1. Handler (sync) stashes `CommandEffect::SetEffortLevelShared(level)`
//!    AND `ctx.pending_effort_set = Some(level)` AND
//!    `try_send(TuiEvent::TextDelta(..))` — effect+sidecar stashed
//!    first, then TextDelta (so the confirmation lands in the TUI
//!    channel before dispatch returns).
//! 2. `apply_effect` (async) awaits
//!    `*slash_ctx.effort_level_shared.lock().await = level` (shared
//!    write).
//! 3. slash.rs dispatch-site sidecar drain calls
//!    `effort_state.set_level(level)` on the local `&mut EffortState`.
//!
//! Both the handler's `try_send` and `apply_effect`'s await complete
//! inside `handle_slash_command` before it returns to the main input
//! loop. The user-observable state at the next input tick is therefore
//! identical: `effort_level_shared` has the new level, `effort_state`
//! has the new level, AND the TextDelta has been enqueued. The only
//! observable drift is the ORDER of the TextDelta vs the state writes —
//! shipped did writes-then-delta, post-migration does delta-then-writes.
//! Because neither the TUI event consumer nor any downstream observer
//! inspects the effort state between the delta and the writes (both
//! land within the same dispatch turn), the drift is invariant-
//! preserving.
//!
//! # R7 TEMPORARY DOUBLE-FIRE NOTE (Gates 1-4 scope)
//!
//! For Gates 1-4 of this ticket the legacy match arm at
//! `src/command/slash.rs:92-122` is LEFT INTACT. Because
//! `dispatcher.dispatch` fires the handler BEFORE the recognized-command
//! short-circuit at slash.rs:61 allows fall-through into the match,
//! `/effort` will fire EffortHandler AND the legacy arm on every input.
//! Mirrors B10-ADDDIR Gates-1-4 double-fire accepted for the same
//! reason. Gate 5 (live-smoke + legacy-arm deletion) removes the double
//! fire in production. Gate 4 for this ticket only runs
//! `cargo test command::effort`, which exercises handler-unit paths
//! under the sync interface and does NOT invoke the legacy arm.
//!
//! # R8 BYTE-IDENTITY PINS
//!
//! Five literal/format strings pinned via `assert_eq!` in the test
//! module:
//!
//! * `description()` — "Show or set reasoning effort (high|medium|low)"
//! * `aliases()` — `&[]`
//! * empty-arg TextDelta — `format!("\nCurrent effort level: {}\nUsage:
//!   /effort <high|medium|low>\n", snapshot_level)`
//! * success TextDelta — `format!("\nEffort level set to {level}.\n")`
//! * validation Error — exact string returned by
//!   `archon_tools::validation::validate_effort_level(level_str)`.

use archon_llm::effort::EffortLevel;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};
use crate::slash_context::SlashCommandContext;

/// Owned snapshot of the single value the /effort READ path needs from
/// shared state. Built at the dispatch site (where `.await` is allowed)
/// and threaded through [`CommandContext`] so the sync handler can
/// consume without holding locks.
///
/// Field is a plain owned [`EffortLevel`] — `Copy`, no `Arc`, no
/// `Mutex`, no borrow.
#[derive(Debug, Clone)]
pub(crate) struct EffortSnapshot {
    /// The current effort level captured at dispatch time by awaiting
    /// `SlashCommandContext::effort_level_shared`. Verified equivalent
    /// to the shipped `effort_state.level()` read because both fields
    /// are mutated in lockstep by the /effort handler.
    pub(crate) current_level: EffortLevel,
}

/// Build an [`EffortSnapshot`] by awaiting the `effort_level_shared`
/// lock in the SAME order and with the SAME value selection as the
/// shipped READ path at `src/command/slash.rs:99`.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/effort`. All other commands leave
/// `effort_snapshot = None` to avoid unnecessary lock traffic.
pub(crate) async fn build_effort_snapshot(
    slash_ctx: &SlashCommandContext,
) -> EffortSnapshot {
    let guard = slash_ctx.effort_level_shared.lock().await;
    let current_level = *guard;
    EffortSnapshot { current_level }
    // Guard drops here — lock released before return.
}

/// Zero-sized handler registered as the primary `/effort` command.
///
/// No aliases (see R3 in module rustdoc). Body-migrate of the shipped
/// arm at slash.rs:92-122 — HYBRID pattern (SNAPSHOT + EFFECT-SLOT +
/// SIDECAR).
///
/// # Behavior
///
/// * Empty args (bare `/effort`) → emit a TextDelta listing the
///   current effort level (from the snapshot) and a usage hint.
/// * Valid effort level (`high`/`medium`/`low`/`med`, case-insensitive,
///   per `validate_effort_level`) → stash BOTH
///   `CommandEffect::SetEffortLevelShared(level)` AND
///   `ctx.pending_effort_set = Some(level)`, then emit a confirmation
///   TextDelta.
/// * Invalid effort level → emit a `TuiEvent::Error` with the
///   byte-identical validator message.
pub(crate) struct EffortHandler;

impl CommandHandler for EffortHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // R4: join multi-token args with " " and trim. Byte-equivalent
        // to the shipped `s.strip_prefix("/effort").unwrap_or("").trim()`
        // for all inputs — single-token levels collapse to the same
        // value as `args.first().unwrap_or("").as_str()`, multi-token
        // inputs preserve the whitespace-joined substring. Empty args
        // and a whitespace-only join both produce the empty string,
        // routing to the help branch identical to the shipped
        // `if level_str.is_empty()` check.
        let joined = args.join(" ");
        let level_str = joined.trim();

        if level_str.is_empty() {
            // READ branch: consume the pre-built snapshot populated
            // by `build_command_context` when the primary resolved to
            // `/effort`. A `None` here indicates a wiring regression
            // (builder bypassed or alias map drifted); surface it as
            // a loud `Err` rather than a user-facing message (mirrors
            // ModelHandler's defensive stance).
            let snap = ctx.effort_snapshot.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "EffortHandler invoked without effort_snapshot populated \
                     — build_command_context bug"
                )
            })?;

            // Byte-for-byte faithful to shipped READ body at
            // slash.rs:97-101. `{}` uses EffortLevel's `Display` impl
            // which yields "high"/"medium"/"low".
            let msg = format!(
                "\nCurrent effort level: {}\nUsage: /effort <high|medium|low>\n",
                snap.current_level
            );
            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(msg));
            return Ok(());
        }

        // WRITE branch: validate, then (on Ok) stash BOTH the shared-
        // mutex effect AND the sidecar slot for local `EffortState`,
        // and emit the confirmation TextDelta. On Err emit
        // TuiEvent::Error byte-for-byte from the validator and do NOT
        // stash either effect or sidecar.
        match archon_tools::validation::validate_effort_level(level_str) {
            Ok(validated) => {
                // `validated` is always one of "high" / "medium" /
                // "low" per validate_effort_level's contract, so
                // parse_level MUST succeed. Any panic here indicates
                // a drift between the validator and the parser and
                // deserves a loud failure (matches shipped
                // `.expect("validated effort level must parse")`).
                let level = archon_llm::effort::parse_level(&validated)
                    .expect("validated effort level must parse");

                // Stash the shared-mutex write (drained by apply_effect
                // at the dispatch site).
                ctx.pending_effect =
                    Some(CommandEffect::SetEffortLevelShared(level));
                // Stash the local EffortState write (drained at the
                // dispatch site AFTER apply_effect, where
                // `&mut effort_state` is in scope).
                ctx.pending_effort_set = Some(level);

                let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(format!(
                    "\nEffort level set to {level}.\n"
                )));
            }
            Err(msg) => {
                // Pass the validator's error string through unchanged.
                let _ = ctx.tui_tx.try_send(TuiEvent::Error(msg));
            }
        }
        Ok(())
    }

    fn description(&self) -> &str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:801 (shipped-wins drift-reconcile).
        "Show or set reasoning effort (high|medium|low)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R3: zero aliases shipped → zero aliases preserved. Pinned by
        // test `effort_handler_aliases_are_empty`.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B11-EFFORT: tests for /effort slash-command
// body-migrate. Uses a local `make_ctx` helper (NOT an extension to
// test_support.rs) — mirrors the pattern established by
// src/command/color.rs (B09) and src/command/add_dir.rs (B10).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel and an
    /// optional [`EffortSnapshot`]. Also exposes the sidecar field
    /// `pending_effort_set` initialised to `None`.
    ///
    /// /effort is a HYBRID handler — the READ branch reads
    /// `effort_snapshot`; the WRITE branch stashes BOTH `pending_effect`
    /// AND `pending_effort_set`. Every other optional field stays
    /// `None`. Mirrors the make_ctx fixtures in color.rs / add_dir.rs.
    fn make_ctx(
        snapshot: Option<EffortSnapshot>,
    ) -> (CommandContext, mpsc::Receiver<TuiEvent>) {
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
                show_thinking: None,
                working_dir: None,
                skill_registry: None,
                denial_snapshot: None,
                effort_snapshot: snapshot,
                permissions_snapshot: None,
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    /// Drain every event currently pending in the channel.
    fn drain(rx: &mut mpsc::Receiver<TuiEvent>) -> Vec<TuiEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        events
    }

    /// The description must match the shipped `declare_handler!` stub
    /// at registry.rs:801 BYTE-FOR-BYTE. AGS-817 shipped-wins rule.
    #[test]
    fn effort_handler_description_byte_identical_to_shipped() {
        let h = EffortHandler;
        assert_eq!(
            h.description(),
            "Show or set reasoning effort (high|medium|low)",
            "EffortHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    /// Shipped `declare_handler!` stub at registry.rs:801 carried no
    /// alias slice — equivalent to `&[]`. AGS-817 shipped-wins rule
    /// preserves zero aliases.
    #[test]
    fn effort_handler_aliases_are_empty() {
        let h = EffortHandler;
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "EffortHandler must have an empty alias slice per B11 R3 \
             (shipped declare_handler! stub had no aliases)"
        );
    }

    /// Bare `/effort` (no args) must emit a single `TuiEvent::TextDelta`
    /// whose payload is byte-identical to the shipped
    /// `"\nCurrent effort level: {snapshot_level}\nUsage: /effort
    /// <high|medium|low>\n"` format. NO `pending_effect` and NO
    /// `pending_effort_set` must be stashed — the empty-arg branch is
    /// read-only.
    #[test]
    fn effort_handler_execute_with_no_args_emits_snapshot_text() {
        let snap = EffortSnapshot {
            current_level: EffortLevel::Medium,
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));
        let h = EffortHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "EffortHandler::execute(no-args) must return Ok(()), got: {res:?}"
        );

        // Neither slot populated on READ branch.
        assert!(
            ctx.pending_effect.is_none(),
            "empty-arg branch must NOT stash a CommandEffect; got: {:?}",
            ctx.pending_effect
        );
        assert!(
            ctx.pending_effort_set.is_none(),
            "empty-arg branch must NOT stash a pending_effort_set; got: {:?}",
            ctx.pending_effort_set
        );

        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "empty-arg branch must emit exactly one event; got: {events:?}"
        );
        let expected = format!(
            "\nCurrent effort level: {}\nUsage: /effort <high|medium|low>\n",
            EffortLevel::Medium
        );
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, &expected,
                    "empty-arg branch TextDelta must match shipped format \
                     byte-for-byte"
                );
            }
            other => panic!(
                "empty-arg branch must emit TuiEvent::TextDelta, got: {other:?}"
            ),
        }
    }

    /// A valid effort level (`"high"`) must:
    /// * Stash `CommandEffect::SetEffortLevelShared(EffortLevel::High)`
    ///   in `pending_effect`.
    /// * Stash `EffortLevel::High` in `pending_effort_set` (SIDECAR).
    /// * Emit a single `TuiEvent::TextDelta` whose payload matches the
    ///   shipped `format!("\nEffort level set to {level}.\n")`
    ///   byte-for-byte.
    /// * Emit NO `TuiEvent::Error`.
    #[test]
    fn effort_handler_execute_with_valid_high_stashes_effect_and_sidecar_and_emits_set_text() {
        // snapshot not needed for WRITE path, pass None.
        let (mut ctx, mut rx) = make_ctx(None);
        let h = EffortHandler;
        let res = h.execute(&mut ctx, &["high".to_string()]);
        assert!(
            res.is_ok(),
            "EffortHandler::execute(valid) must return Ok(()), got: {res:?}"
        );

        // 1. pending_effect MUST be Some(SetEffortLevelShared(High)).
        match ctx.pending_effect.as_ref() {
            Some(CommandEffect::SetEffortLevelShared(level)) => {
                assert_eq!(
                    *level,
                    EffortLevel::High,
                    "SetEffortLevelShared must carry the parsed EffortLevel"
                );
            }
            other => panic!(
                "expected Some(CommandEffect::SetEffortLevelShared(High)), \
                 got: {other:?}"
            ),
        }

        // 2. pending_effort_set SIDECAR MUST be Some(High).
        assert_eq!(
            ctx.pending_effort_set,
            Some(EffortLevel::High),
            "pending_effort_set sidecar must carry the parsed EffortLevel"
        );

        // 3. Exactly one TextDelta event with byte-identical format.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "valid-arg branch must emit exactly one event; got: {events:?}"
        );
        let expected = format!("\nEffort level set to {}.\n", EffortLevel::High);
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, &expected,
                    "valid-arg branch TextDelta must match shipped \
                     format! byte-for-byte"
                );
            }
            other => panic!(
                "valid-arg branch must emit TuiEvent::TextDelta, got: {other:?}"
            ),
        }

        // 4. NO Error event.
        let has_error = events
            .iter()
            .any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            !has_error,
            "valid-arg branch must emit NO TuiEvent::Error; got: {events:?}"
        );
    }

    /// An invalid effort level must:
    /// * Emit a single `TuiEvent::Error` whose payload is byte-
    ///   identical to the string returned by
    ///   `archon_tools::validation::validate_effort_level(level_str)`.
    /// * NOT stash any `CommandEffect` (pending_effect remains None).
    /// * NOT stash the sidecar (pending_effort_set remains None).
    /// * NOT emit any `TuiEvent::TextDelta`.
    #[test]
    fn effort_handler_execute_with_invalid_arg_emits_validation_error() {
        let (mut ctx, mut rx) = make_ctx(None);
        let h = EffortHandler;
        let bogus = "turbo";
        // Capture the validator's exact error message so we pin the
        // byte-identical pass-through. Any future change to the
        // validator would need to update this expectation in lockstep —
        // that is the intended coupling.
        let expected_msg = archon_tools::validation::validate_effort_level(bogus)
            .expect_err("'turbo' must NOT be a valid effort level");

        let res = h.execute(&mut ctx, &[bogus.to_string()]);
        assert!(
            res.is_ok(),
            "EffortHandler::execute(invalid) must return Ok(()), got: {res:?}"
        );

        // 1. NO effect stashed.
        assert!(
            ctx.pending_effect.is_none(),
            "invalid-arg branch must NOT stash a CommandEffect; got: {:?}",
            ctx.pending_effect
        );
        // 2. NO sidecar stashed.
        assert!(
            ctx.pending_effort_set.is_none(),
            "invalid-arg branch must NOT stash a pending_effort_set; got: {:?}",
            ctx.pending_effort_set
        );

        // 3. Exactly one Error event with byte-identical payload.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "invalid-arg branch must emit exactly one event; got: {events:?}"
        );
        match &events[0] {
            TuiEvent::Error(msg) => {
                assert_eq!(
                    msg, &expected_msg,
                    "invalid-arg branch Error must match validate_effort_level \
                     output byte-for-byte (pass-through)"
                );
            }
            other => panic!(
                "invalid-arg branch must emit TuiEvent::Error, got: {other:?}"
            ),
        }
        // 4. NO TextDelta.
        let has_delta = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(_)));
        assert!(
            !has_delta,
            "invalid-arg branch must emit NO TuiEvent::TextDelta; got: {events:?}"
        );
    }

    /// Defensive test for R4: passing a multi-token args slice (e.g.
    /// `["high", "extra"]`) must:
    /// * Join with " " and trim to `"high extra"`.
    /// * Not panic.
    /// * Return `Ok(())`.
    /// * Emit at least one event (almost certainly an Error because
    ///   `validate_effort_level("high extra")` fails; but a future
    ///   validator that accepted multi-word levels would satisfy this
    ///   test equally via the success branch).
    #[test]
    fn effort_handler_execute_joins_multi_token_args_without_panicking() {
        let (mut ctx, mut rx) = make_ctx(None);
        let h = EffortHandler;
        let args = vec!["high".to_string(), "extra".to_string()];
        let res = h.execute(&mut ctx, &args);
        assert!(
            res.is_ok(),
            "EffortHandler::execute(multi-token) must return Ok(()), got: {res:?}"
        );

        let events = drain(&mut rx);
        assert!(
            !events.is_empty(),
            "EffortHandler::execute(multi-token) must emit at least one \
             event; got: {events:?}"
        );
    }

    // -------------------------------------------------------------------
    // Gate 5 dispatcher-integration tests — TASK-AGS-POST-6-BODIES-B11-EFFORT
    // -------------------------------------------------------------------
    //
    // These tests drive the real `Dispatcher` + `default_registry()` +
    // `EffortHandler` end-to-end, replacing the unit-level `h.execute(...)`
    // harness with the same dispatch path the TUI input loop uses. They
    // pin the fact that (a) registry routing for "/effort" lands on
    // `EffortHandler`, (b) parser tokenization delivers args correctly
    // for both bare and trailing-args forms, (c) byte-framing of shipped
    // strings survives the full dispatch chain, and (d) the HYBRID
    // pattern's three slots (effort_snapshot READ, pending_effect
    // ASYNC-WRITE, pending_effort_set SIDECAR) wire correctly through
    // the dispatcher.
    //
    // Reference template: src/command/add_dir.rs dispatcher tests
    // (B10-ADDDIR Gate 5) — same structure. Zero mocks: Arc<Registry>
    // from `default_registry()` + `Dispatcher::new` exactly as the live
    // harness builds them in session.rs.

    #[test]
    fn dispatcher_routes_slash_effort_to_handler_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);

        // Bare "/effort" → READ branch → TextDelta with snapshot level.
        // The handler reads `effort_snapshot` (populated here inline —
        // in production the builder fills it before dispatch). Use
        // Medium as the harness default.
        let snap = EffortSnapshot {
            current_level: EffortLevel::Medium,
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));

        let result = dispatcher.dispatch(&mut ctx, "/effort");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/effort\") must return Ok; got: {result:?}"
        );

        // 1. NO pending_effect (empty-arg branch is READ-only).
        assert!(
            ctx.pending_effect.is_none(),
            "end-to-end bare `/effort` must NOT stash a CommandEffect; \
             got: {:?}",
            ctx.pending_effect
        );
        // 2. NO pending_effort_set sidecar (empty-arg branch is READ-only).
        assert!(
            ctx.pending_effort_set.is_none(),
            "end-to-end bare `/effort` must NOT stash a pending_effort_set \
             sidecar; got: {:?}",
            ctx.pending_effort_set
        );

        // 3. Exactly one TextDelta whose payload is byte-identical to
        //    the shipped format!() output for the snapshot level.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "end-to-end bare `/effort` must emit exactly one event; got: \
             {events:?}"
        );
        let expected = format!(
            "\nCurrent effort level: {}\nUsage: /effort <high|medium|low>\n",
            EffortLevel::Medium
        );
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, &expected,
                    "end-to-end bare `/effort` TextDelta must match shipped \
                     format! byte-for-byte"
                );
            }
            other => panic!(
                "end-to-end bare `/effort` must emit TuiEvent::TextDelta, \
                 got: {other:?}"
            ),
        }

        // 4. NO Error event.
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            !has_error,
            "end-to-end bare `/effort` must emit NO TuiEvent::Error; got: \
             {events:?}"
        );
    }

    #[test]
    fn dispatcher_routes_slash_effort_with_high_arg_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use crate::command::registry::CommandEffect;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        // snapshot not needed for WRITE branch, pass None.
        let (mut ctx, mut rx) = make_ctx(None);

        // "/effort high" → WRITE branch → effect stash + sidecar stash +
        // TextDelta confirmation.
        let result = dispatcher.dispatch(&mut ctx, "/effort high");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/effort high\") must return Ok; got: \
             {result:?}"
        );

        // 1. pending_effect MUST be Some(SetEffortLevelShared(High)).
        match ctx.pending_effect.as_ref() {
            Some(CommandEffect::SetEffortLevelShared(level)) => {
                assert_eq!(
                    *level,
                    EffortLevel::High,
                    "SetEffortLevelShared must carry the parsed EffortLevel \
                     from the dispatched arg"
                );
            }
            other => panic!(
                "expected Some(CommandEffect::SetEffortLevelShared(High)), \
                 got: {other:?}"
            ),
        }

        // 2. pending_effort_set SIDECAR MUST be Some(High).
        assert_eq!(
            ctx.pending_effort_set,
            Some(EffortLevel::High),
            "pending_effort_set sidecar must carry the parsed EffortLevel \
             from the dispatched arg"
        );

        // 3. Exactly one TextDelta whose payload is byte-identical to
        //    the shipped success format!() output.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "end-to-end `/effort high` must emit exactly one event; got: \
             {events:?}"
        );
        let expected = format!("\nEffort level set to {}.\n", EffortLevel::High);
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, &expected,
                    "end-to-end `/effort high` TextDelta must match shipped \
                     format! byte-for-byte"
                );
            }
            other => panic!(
                "end-to-end `/effort high` must emit TuiEvent::TextDelta, \
                 got: {other:?}"
            ),
        }

        // 4. NO Error event.
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            !has_error,
            "end-to-end `/effort high` must emit NO TuiEvent::Error; got: \
             {events:?}"
        );
    }
}
