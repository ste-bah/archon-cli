//! TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: /permissions slash-command
//! handler (body-migrate, HYBRID pattern — SNAPSHOT + EFFECT-SLOT, NO
//! sidecar).
//!
//! Reference: shipped inline match arm at `src/command/slash.rs:295-336`.
//! Source:   shipped `declare_handler!(PermissionsHandler, "Show or
//!           update tool permissions")` stub at
//!           `src/command/registry.rs:914` (no aliases).
//!
//! # R1 PATTERN-CONFIRM (HYBRID chosen)
//!
//! The shipped body at slash.rs:295-336 performs THREE actions that
//! cannot all run inside a sync `CommandHandler::execute`:
//!
//! 1. **Async READ** `ctx.permission_mode.lock().await` — for the
//!    empty-arg display branch ("Current permission mode: {mode}").
//! 2. **Sync READ** `ctx.allow_bypass_permissions: bool` — for the
//!    bypass-allow guard when validated == "bypassPermissions".
//! 3. **Async WRITE** `*ctx.permission_mode.lock().await = resolved` —
//!    for the valid-set path, followed by emitting
//!    `TuiEvent::PermissionModeChanged(resolved)` AFTER the write and
//!    a confirmation `TextDelta` after the event.
//!
//! No single existing pattern accommodates all three. HYBRID =
//! SNAPSHOT (AGS-807/808/B08/B11 precedent) + EFFECT-SLOT (AGS-808/
//! B04-DIFF/B10-ADDDIR/B11-EFFORT precedent). NO sidecar is required —
//! unlike /effort, `/permissions` has no session-local stack state to
//! mutate. The snapshot carries BOTH required values: the async-locked
//! `current_mode: String` AND the sync-read `allow_bypass_permissions:
//! bool`. Bundling them into one snapshot keeps the extension surface
//! minimal (one new snapshot field on `CommandContext`, one new
//! `CommandEffect` variant) and matches the AGS-808 `ModelSnapshot`
//! shape for a single-primary handler.
//!
//! * **READ side → SNAPSHOT pattern**. A new
//!   `permissions_snapshot: Option<PermissionsSnapshot>` field on
//!   `CommandContext` is populated by `build_command_context` ONLY
//!   when the primary resolves to `/permissions`. The builder awaits
//!   `slash_ctx.permission_mode.lock().await` and captures
//!   `slash_ctx.allow_bypass_permissions` (sync) into the owned
//!   snapshot so the sync handler can render the current-mode line
//!   AND guard the bypass branch without any locking.
//!
//! * **ASYNC WRITE side → EFFECT-SLOT pattern**. A new
//!   `CommandEffect::SetPermissionMode(String)` variant. The sync
//!   handler stashes the effect via `ctx.pending_effect`.
//!   `apply_effect` in `src/command/context.rs` awaits
//!   `*slash_ctx.permission_mode.lock().await = resolved` AND emits
//!   `TuiEvent::PermissionModeChanged(resolved)` via
//!   `tui_tx.send(..).await` (apply_effect is async, so .await is
//!   legal — and the event MUST be awaited to match the shipped
//!   emission-after-write ordering at slash.rs:320-323).
//!
//! # R2 PRIMARY-ALREADY-REGISTERED
//!
//! `permissions` is already a primary in the default registry via the
//! `declare_handler!(PermissionsHandler, "Show or update tool
//! permissions")` stub at registry.rs:914 (no aliases). This ticket is a
//! body-migrate, NOT a gap-fix: primary count is UNCHANGED. The stub is
//! REMOVED in favour of the real type defined in this file, imported
//! into registry.rs at the top via
//! `use crate::command::permissions::PermissionsHandler;`.
//!
//! # R3 NO-ALIASES (shipped-wins drift-reconcile)
//!
//! Shipped `declare_handler!` stub at registry.rs:914 used the two-arg
//! form — equivalent to `&[]`. AGS-817 shipped-wins drift-reconcile
//! rule preserves zero aliases. This handler returns `&[]` from
//! `aliases()` and the test `permissions_handler_aliases_are_empty`
//! pins the invariant against silent additions.
//!
//! # R4 ARGS-RECONCILIATION
//!
//! Shipped body uses `s.strip_prefix("/permissions").unwrap_or("").trim()`
//! on the raw input string — a single-string substring after the
//! command name. The parser tokenizes on whitespace into
//! `args: &[String]`. For a single-token mode (`/permissions plan`),
//! `args.first()` would be byte-equivalent. The handler uses
//! `args.join(" ").trim()` which preserves the shipped substring
//! semantics EXACTLY for any multi-token input and degrades gracefully
//! to the same single-token form for the common case. Empty args
//! (bare `/permissions`) and a whitespace-only join both produce the
//! empty string, routing to the help branch identical to the shipped
//! `if arg.is_empty()` check. Mirrors B11 R4.
//!
//! # R5 EMISSION-PRIMITIVE-SWAP (.await -> try_send, with apply_effect
//! caveat)
//!
//! Shipped body emitted via `tui_tx.send(..).await` — async, blocking
//! on backpressure if the 16-cap channel is full. The sync
//! `CommandHandler::execute` signature cannot `.await`, so the HANDLER
//! uses `ctx.tui_tx.try_send(..)` (sync, best-effort drop on full) for
//! the empty-arg TextDelta, the bypass-blocked Error, the confirmation
//! TextDelta, and the invalid Error pass-through. APPLY_EFFECT is
//! async and therefore uses `.send(..).await` for the
//! `PermissionModeChanged` event — this preserves the shipped
//! emission-after-write ordering without introducing drop-on-full
//! drift for a state-change notification.
//!
//! Four format strings preserved BYTE-FOR-BYTE:
//!
//! 1. Empty-arg TextDelta — `format!("\nCurrent permission mode:
//!    {mode}\nUsage: /permissions <mode>\nModes: default, acceptEdits,
//!    plan, auto, dontAsk, bypassPermissions\nLegacy aliases: ask ->
//!    default, yolo -> bypassPermissions\n")`. The shipped source uses
//!    `\` line-continuations; the actual concatenated string in memory
//!    has NO leading spaces between sections (verified against
//!    slash.rs:301-306).
//! 2. Bypass-blocked Error (validated == "bypassPermissions" AND
//!    allow_bypass_permissions == false) —
//!    `"bypassPermissions requires --allow-dangerously-skip-permissions flag"`.
//! 3. Set TextDelta — `format!("\nPermission mode set to {resolved}.\n")`.
//! 4. Invalid Error — pass-through from
//!    `archon_tools::validation::validate_permission_mode(arg)`
//!    `Err(msg)` byte-for-byte (no wrapping, no rewrite).
//!
//! 5. `TuiEvent::PermissionModeChanged(resolved)` — pass-through of
//!    the resolved String. Emitted by `apply_effect` AFTER the mutex
//!    write.
//!
//! # R6 ORDER-SEMANTICS-SWAP (accepted, matches B10/B11)
//!
//! Shipped order at slash.rs:319-328:
//!
//! ```ignore
//! *ctx.permission_mode.lock().await = resolved.clone();       // 1. write
//! tui_tx.send(TuiEvent::PermissionModeChanged(...)).await;    // 2. event
//! tui_tx.send(TuiEvent::TextDelta("Permission mode set..."))  // 3. delta
//!     .await;
//! ```
//!
//! Post-migration order:
//!
//! 1. Handler (sync) `try_send(TuiEvent::TextDelta("Permission mode
//!    set to {resolved}."))` — confirmation FIRST, matching B10/B11
//!    emission-order swap (TextDelta before effect stash).
//! 2. Handler stashes `CommandEffect::SetPermissionMode(resolved)` via
//!    `ctx.pending_effect`.
//! 3. `apply_effect` (async): `*slash_ctx.permission_mode.lock().await
//!    = resolved` (shared write).
//! 4. `apply_effect` (async): `tui_tx.send(TuiEvent::
//!    PermissionModeChanged(resolved)).await` — state-change
//!    notification AFTER the write, preserving shipped ordering of
//!    write-then-notify.
//!
//! Net effect on user-observable state at the next input tick:
//! `permission_mode` holds the new mode, the `PermissionModeChanged`
//! event has been consumed by the TUI event loop, AND the
//! confirmation TextDelta has been enqueued. The only observable
//! drift is the ORDER of the TextDelta vs the state write — shipped
//! did write-then-event-then-delta; post-migration does
//! delta-then-write-then-event. Because neither the TUI event
//! consumer nor any downstream observer inspects the permission mode
//! between the delta and the write (both land within the same
//! dispatch turn), the drift is invariant-preserving. Mirrors
//! B10-ADDDIR and B11-EFFORT accepted order swaps.
//!
//! # R7 TEMPORARY DOUBLE-FIRE NOTE (Gates 1-4 scope)
//!
//! For Gates 1-4 of this ticket the legacy match arm at
//! `src/command/slash.rs:295-336` is LEFT INTACT. Because
//! `dispatcher.dispatch` fires the handler BEFORE the recognized-command
//! short-circuit at slash.rs:61 allows fall-through into the match,
//! `/permissions` will fire PermissionsHandler AND the legacy arm on
//! every input. Mirrors B10-ADDDIR / B11-EFFORT Gates-1-4 double-fire
//! accepted for the same reason. Gate 5 (live-smoke + legacy-arm
//! deletion) removes the double fire in production. Gate 4 for this
//! ticket only runs `cargo test command::permissions`, which exercises
//! handler-unit paths under the sync interface and does NOT invoke the
//! legacy arm.

use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};
use crate::slash_context::SlashCommandContext;
use archon_tui::app::TuiEvent;

/// Owned snapshot of the two values the /permissions handler needs from
/// shared state. Built at the dispatch site (where `.await` is allowed)
/// and threaded through [`CommandContext`] so the sync handler can
/// consume without holding locks.
///
/// Carries BOTH:
/// * `current_mode: String` — captured from the async
///   `SlashCommandContext::permission_mode` mutex. Used by the empty-arg
///   display branch.
/// * `allow_bypass_permissions: bool` — copied from
///   `SlashCommandContext::allow_bypass_permissions` (plain `bool`).
///   Used by the bypass-allow guard when `validated == "bypassPermissions"`.
///
/// Bundling the two fields into one snapshot (rather than adding a
/// second DIRECT field on `CommandContext`) keeps the extension
/// surface minimal — one snapshot per primary, no cross-cutting field.
#[derive(Debug, Clone)]
pub(crate) struct PermissionsSnapshot {
    /// The current permission mode captured at dispatch time by
    /// awaiting `SlashCommandContext::permission_mode`.
    pub(crate) current_mode: String,
    /// The `[permissions]` rules in force, as `(effect, tool, pattern)`
    /// with effect one of `deny`, `allow`, `ask` (#192).
    ///
    /// Deny first, then allow, then ask — the order the checker evaluates
    /// them, so the list reads the way the decision is made.
    pub(crate) rules: Vec<(String, String, String)>,
    /// Whether `--allow-dangerously-skip-permissions` was passed on the
    /// CLI; unlocks the `bypassPermissions` mode.
    pub(crate) allow_bypass_permissions: bool,
}

/// Build a [`PermissionsSnapshot`] by awaiting the `permission_mode`
/// lock in the SAME order as the shipped READ path at
/// `src/command/slash.rs:299` and copying the sync `bool`
/// `allow_bypass_permissions` field.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/permissions`. All other commands leave
/// `permissions_snapshot = None` to avoid unnecessary lock traffic.
pub(crate) async fn build_permissions_snapshot(
    slash_ctx: &SlashCommandContext,
) -> PermissionsSnapshot {
    let guard = slash_ctx.permission_mode.lock().await;
    let current_mode = guard.clone();
    drop(guard); // Guard dropped before return (explicit for clarity).

    let rules = &slash_ctx.permission_rules;
    let flatten = |effect: &'static str, entries: &[archon_permissions::rules::ToolRule]| {
        entries
            .iter()
            .map(|rule| (effect.to_string(), rule.tool.clone(), rule.pattern.clone()))
            .collect::<Vec<_>>()
    };
    let mut flattened = flatten("deny", &rules.always_deny);
    flattened.extend(flatten("allow", &rules.always_allow));
    flattened.extend(flatten("ask", &rules.always_ask));

    PermissionsSnapshot {
        current_mode,
        rules: flattened,
        allow_bypass_permissions: slash_ctx.allow_bypass_permissions,
    }
}

/// Zero-sized handler registered as the primary `/permissions` command.
///
/// No aliases (see R3 in module rustdoc). Body-migrate of the shipped
/// arm at slash.rs:295-336 — HYBRID pattern (SNAPSHOT + EFFECT-SLOT, NO
/// sidecar).
///
/// # Behavior
///
/// * Empty args (bare `/permissions`) → emit a TextDelta listing the
///   current permission mode (from the snapshot), usage hint, valid
///   modes, and legacy aliases.
/// * `bypassPermissions` when `!allow_bypass_permissions` → emit a
///   single `TuiEvent::Error` with the byte-identical guard message.
/// * Valid permission mode (via `validate_permission_mode`) → emit a
///   confirmation `TextDelta` THEN stash
///   `CommandEffect::SetPermissionMode(resolved)`. `apply_effect`
///   performs the mutex write and emits `PermissionModeChanged`.
/// * Invalid permission mode → emit a `TuiEvent::Error` with the
///   byte-identical validator message.
pub(crate) struct PermissionsHandler;

impl CommandHandler for PermissionsHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // R4: join multi-token args with " " and trim. Byte-equivalent
        // to the shipped `s.strip_prefix("/permissions").unwrap_or("").trim()`
        // for all inputs — single-token modes collapse to the same
        // value as `args.first().unwrap_or("").as_str()`, multi-token
        // inputs preserve the whitespace-joined substring. Empty args
        // and a whitespace-only join both produce the empty string,
        // routing to the help branch identical to the shipped
        // `if arg.is_empty()` check.
        let joined = args.join(" ");
        let arg = joined.trim();

        if arg.is_empty() {
            // READ branch: consume the pre-built snapshot populated
            // by `build_command_context` when the primary resolved to
            // `/permissions`. A `None` here indicates a wiring
            // regression (builder bypassed or alias map drifted);
            // surface it as a loud `Err` rather than a user-facing
            // message (mirrors ModelHandler/EffortHandler defensive
            // stance).
            let snap = ctx.permissions_snapshot.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "PermissionsHandler invoked without permissions_snapshot \
                     populated — build_command_context bug"
                )
            })?;

            // Byte-for-byte faithful to shipped READ body at
            // slash.rs:300-307. The `\` line-continuations in shipped
            // source eat whitespace up to the next non-whitespace, so
            // the actual concatenated string has NO leading spaces
            // between sections.
            let mode = &snap.current_mode;
            let msg = format!(
                "\nCurrent permission mode: {mode}\nUsage: /permissions <mode>\nModes: default, acceptEdits, plan, auto, dontAsk, bypassPermissions\nLegacy aliases: ask -> default, yolo -> bypassPermissions\n"
            );
            ctx.emit(TuiEvent::TextDelta(msg));

            // #192: and the rules the mode is qualified by. The mode line has
            // never said whether an always-deny entry overrides it, and there
            // was nowhere else to look. Additive — the text above is
            // unchanged and print mode drops this event.
            ctx.emit(TuiEvent::ShowPermissions {
                mode: mode.clone(),
                rules: snap.rules.clone(),
            });
            return Ok(());
        }

        // WRITE branch: validate, then enter one of three sub-paths:
        //   - Ok(resolved) + bypassPermissions + !allow_bypass_permissions
        //     → bypass-blocked Error, NO effect stash.
        //   - Ok(resolved) (any other case) → confirmation TextDelta,
        //     stash SetPermissionMode(resolved).
        //   - Err(msg) → pass-through Error, NO effect stash.
        match archon_tools::validation::validate_permission_mode(arg) {
            Ok(resolved) => {
                // Re-read snapshot for the bypass-allow guard. If
                // snapshot is missing here (defensive), surface it as
                // Err since the bypass decision cannot be made safely.
                let snap = ctx.permissions_snapshot.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "PermissionsHandler invoked without permissions_snapshot \
                         populated — build_command_context bug"
                    )
                })?;

                if resolved == "bypassPermissions" && !snap.allow_bypass_permissions {
                    crate::runtime::permission_events::record_permission_mode_event(
                        ctx.governed_learning_db.as_ref(),
                        ctx.session_id.as_deref(),
                        Some(&snap.current_mode),
                        &resolved,
                        "mode_change_denied",
                        "dangerous_bypass_guard",
                    );
                    // Bypass-blocked branch — byte-identical error
                    // string from shipped slash.rs:315.
                    ctx.emit(TuiEvent::Error(
                        "bypassPermissions requires --allow-dangerously-skip-permissions flag"
                            .into(),
                    ));
                    return Ok(());
                }

                // Emit confirmation TextDelta BEFORE stashing the
                // effect (matches B10/B11 emission-order swap — see R6
                // in module rustdoc).
                ctx.emit(TuiEvent::TextDelta(format!(
                    "\nPermission mode set to {resolved}.\n"
                )));

                // Stash the shared-mutex write (drained by apply_effect
                // at the dispatch site). apply_effect ALSO emits the
                // PermissionModeChanged event AFTER the write.
                ctx.pending_effect = Some(CommandEffect::SetPermissionMode(resolved));
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
        // stub at registry.rs:914 (shipped-wins drift-reconcile).
        "Show or update tool permissions"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R3: zero aliases shipped → zero aliases preserved. Pinned by
        // test `permissions_handler_aliases_are_empty`.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B12-PERMISSIONS: tests for /permissions
// slash-command body-migrate. Uses a local `make_ctx` helper (NOT an
// extension to test_support.rs) — mirrors the pattern established by
// src/command/effort.rs (B11) and src/command/add_dir.rs (B10).
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "permissions_tests.rs"]
mod tests;
