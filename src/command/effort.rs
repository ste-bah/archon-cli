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
//! shipped format strings are preserved BYTE-FOR-BYTE, except for the
//! tier list, which #123 widened from `high|medium|low` to
//! `low|medium|high|max` in both the usage line and `description()`:
//!
//! 1. `"\nCurrent effort level: {}\nUsage: /effort <low|medium|high|max>\n"`
//!    (empty-arg branch — `{}` is the snapshot level's `Display`
//!    impl, which yields `"low"` / `"medium"` / `"high"` / `"max"`).
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
//! * `description()` — "Show or set reasoning effort (low|medium|high|max)"
//! * `aliases()` — `&[]`
//! * empty-arg TextDelta — `format!("\nCurrent effort level: {}\nUsage:
//!   /effort <low|medium|high|max>\n", snapshot_level)`
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
pub(crate) async fn build_effort_snapshot(slash_ctx: &SlashCommandContext) -> EffortSnapshot {
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
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
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
                "\nCurrent effort level: {}\nUsage: /effort <low|medium|high|max>\n",
                snap.current_level
            );
            ctx.emit(TuiEvent::TextDelta(msg));
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
                ctx.pending_effect = Some(CommandEffect::SetEffortLevelShared(level));
                // Stash the local EffortState write (drained at the
                // dispatch site AFTER apply_effect, where
                // `&mut effort_state` is in scope).
                ctx.pending_effort_set = Some(level);

                ctx.emit(TuiEvent::TextDelta(format!(
                    "\nEffort level set to {level}.\n"
                )));
            }
            Err(msg) => {
                // Pass the validator's error string through unchanged.
                ctx.emit(TuiEvent::Error(msg));
            }
        }
        Ok(())
    }

    fn description(&self) -> &str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:801 (shipped-wins drift-reconcile).
        "Show or set reasoning effort (low|medium|high|max)"
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
#[path = "effort_tests.rs"]
mod tests;
