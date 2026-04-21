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

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};
use crate::slash_context::SlashCommandContext;

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
    PermissionsSnapshot {
        current_mode,
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
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
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
            let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(msg));
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
                    // Bypass-blocked branch — byte-identical error
                    // string from shipped slash.rs:315.
                    let _ = ctx.tui_tx.try_send(TuiEvent::Error(
                        "bypassPermissions requires --allow-dangerously-skip-permissions flag"
                            .into(),
                    ));
                    return Ok(());
                }

                // Emit confirmation TextDelta BEFORE stashing the
                // effect (matches B10/B11 emission-order swap — see R6
                // in module rustdoc).
                let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(format!(
                    "\nPermission mode set to {resolved}.\n"
                )));

                // Stash the shared-mutex write (drained by apply_effect
                // at the dispatch site). apply_effect ALSO emits the
                // PermissionModeChanged event AFTER the write.
                ctx.pending_effect =
                    Some(CommandEffect::SetPermissionMode(resolved));
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
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel and an
    /// optional [`PermissionsSnapshot`]. All other optional fields
    /// stay `None`. Mirrors the make_ctx fixtures in effort.rs /
    /// add_dir.rs.
    fn make_ctx(
        snapshot: Option<PermissionsSnapshot>,
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
                effort_snapshot: None,
                permissions_snapshot: snapshot,
                copy_snapshot: None,
                doctor_snapshot: None,
                usage_snapshot: None,
                config_path: None,
                auth_label: None,
                pending_effect: None,
                pending_effort_set: None,
                pending_export: None,
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
    /// at registry.rs:914 BYTE-FOR-BYTE. AGS-817 shipped-wins rule.
    #[test]
    fn permissions_handler_description_byte_identical_to_shipped() {
        let h = PermissionsHandler;
        assert_eq!(
            h.description(),
            "Show or update tool permissions",
            "PermissionsHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    /// Shipped `declare_handler!` stub at registry.rs:914 used the
    /// two-arg form — equivalent to `&[]`. AGS-817 shipped-wins rule
    /// preserves zero aliases.
    #[test]
    fn permissions_handler_aliases_are_empty() {
        let h = PermissionsHandler;
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "PermissionsHandler must have an empty alias slice per B12 R3 \
             (shipped declare_handler! stub had no aliases)"
        );
    }

    /// Bare `/permissions` (no args) must emit a single
    /// `TuiEvent::TextDelta` whose payload is byte-identical to the
    /// shipped multi-line format with the snapshot's current_mode
    /// interpolated. NO `pending_effect` must be stashed — the
    /// empty-arg branch is read-only.
    #[test]
    fn permissions_handler_execute_with_no_args_emits_snapshot_text() {
        let snap = PermissionsSnapshot {
            current_mode: "default".to_string(),
            allow_bypass_permissions: false,
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));
        let h = PermissionsHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "PermissionsHandler::execute(no-args) must return Ok(()), got: {res:?}"
        );

        assert!(
            ctx.pending_effect.is_none(),
            "empty-arg branch must NOT stash a CommandEffect; got: {:?}",
            ctx.pending_effect
        );

        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "empty-arg branch must emit exactly one event; got: {events:?}"
        );
        let expected =
            "\nCurrent permission mode: default\nUsage: /permissions <mode>\nModes: default, acceptEdits, plan, auto, dontAsk, bypassPermissions\nLegacy aliases: ask -> default, yolo -> bypassPermissions\n"
                .to_string();
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

    /// A valid non-bypass mode (`"plan"`) must:
    /// * Emit a single `TuiEvent::TextDelta` whose payload matches the
    ///   shipped `format!("\nPermission mode set to {resolved}.\n")`
    ///   byte-for-byte.
    /// * Stash `CommandEffect::SetPermissionMode("plan")` in
    ///   `pending_effect`.
    /// * Emit NO `TuiEvent::Error`.
    /// * NOT emit `PermissionModeChanged` (that is apply_effect's job).
    #[test]
    fn permissions_handler_execute_with_valid_plan_stashes_effect_and_emits_set_text() {
        let snap = PermissionsSnapshot {
            current_mode: "default".to_string(),
            allow_bypass_permissions: false,
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));
        let h = PermissionsHandler;
        let res = h.execute(&mut ctx, &["plan".to_string()]);
        assert!(
            res.is_ok(),
            "PermissionsHandler::execute(valid) must return Ok(()), got: {res:?}"
        );

        // 1. pending_effect MUST be Some(SetPermissionMode("plan")).
        match ctx.pending_effect.as_ref() {
            Some(CommandEffect::SetPermissionMode(s)) => {
                assert_eq!(
                    s, "plan",
                    "SetPermissionMode must carry the validated mode string"
                );
            }
            other => panic!(
                "expected Some(CommandEffect::SetPermissionMode(\"plan\")), \
                 got: {other:?}"
            ),
        }

        // 2. Exactly one TextDelta event with byte-identical format.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "valid-arg branch must emit exactly one event (TextDelta); \
             PermissionModeChanged is emitted by apply_effect, not the \
             handler. got: {events:?}"
        );
        let expected = "\nPermission mode set to plan.\n".to_string();
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
    }

    /// `bypassPermissions` with `allow_bypass_permissions == false`
    /// must:
    /// * Emit a single `TuiEvent::Error` with the byte-identical
    ///   guard message.
    /// * NOT stash any `CommandEffect`.
    /// * NOT emit any `TuiEvent::TextDelta`.
    #[test]
    fn permissions_handler_execute_bypass_without_allow_emits_error_no_effect() {
        let snap = PermissionsSnapshot {
            current_mode: "default".to_string(),
            allow_bypass_permissions: false,
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));
        let h = PermissionsHandler;
        let res = h.execute(&mut ctx, &["bypassPermissions".to_string()]);
        assert!(
            res.is_ok(),
            "PermissionsHandler::execute(bypass-blocked) must return \
             Ok(()), got: {res:?}"
        );

        // 1. NO effect stashed.
        assert!(
            ctx.pending_effect.is_none(),
            "bypass-blocked branch must NOT stash a CommandEffect; got: {:?}",
            ctx.pending_effect
        );

        // 2. Exactly one Error event with byte-identical payload.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "bypass-blocked branch must emit exactly one event; got: {events:?}"
        );
        match &events[0] {
            TuiEvent::Error(msg) => {
                assert_eq!(
                    msg,
                    "bypassPermissions requires --allow-dangerously-skip-permissions flag",
                    "bypass-blocked Error must be byte-identical to shipped"
                );
            }
            other => panic!(
                "bypass-blocked branch must emit TuiEvent::Error, got: {other:?}"
            ),
        }
        // 3. NO TextDelta.
        let has_delta = events.iter().any(|e| matches!(e, TuiEvent::TextDelta(_)));
        assert!(
            !has_delta,
            "bypass-blocked branch must emit NO TuiEvent::TextDelta; got: {events:?}"
        );
    }

    /// `bypassPermissions` with `allow_bypass_permissions == true`
    /// must: bypass-allow succeed (fall through the normal valid
    /// path): emit the confirmation TextDelta and stash the effect.
    #[test]
    fn permissions_handler_execute_bypass_with_allow_stashes_effect() {
        let snap = PermissionsSnapshot {
            current_mode: "default".to_string(),
            allow_bypass_permissions: true,
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));
        let h = PermissionsHandler;
        let res = h.execute(&mut ctx, &["bypassPermissions".to_string()]);
        assert!(
            res.is_ok(),
            "PermissionsHandler::execute(bypass-allowed) must return \
             Ok(()), got: {res:?}"
        );

        // 1. Effect MUST be stashed.
        match ctx.pending_effect.as_ref() {
            Some(CommandEffect::SetPermissionMode(s)) => {
                assert_eq!(
                    s, "bypassPermissions",
                    "SetPermissionMode must carry 'bypassPermissions' when allowed"
                );
            }
            other => panic!(
                "expected Some(SetPermissionMode(\"bypassPermissions\")), \
                 got: {other:?}"
            ),
        }

        // 2. Confirmation TextDelta.
        let events = drain(&mut rx);
        assert_eq!(events.len(), 1, "expected one TextDelta; got: {events:?}");
        let expected = "\nPermission mode set to bypassPermissions.\n".to_string();
        match &events[0] {
            TuiEvent::TextDelta(text) => assert_eq!(text, &expected),
            other => panic!("expected TextDelta, got: {other:?}"),
        }
    }

    /// An invalid mode must:
    /// * Emit a single `TuiEvent::Error` whose payload is byte-
    ///   identical to the string returned by
    ///   `archon_tools::validation::validate_permission_mode(arg)`.
    /// * NOT stash any `CommandEffect`.
    /// * NOT emit any `TuiEvent::TextDelta`.
    #[test]
    fn permissions_handler_execute_with_invalid_arg_emits_validation_error() {
        let snap = PermissionsSnapshot {
            current_mode: "default".to_string(),
            allow_bypass_permissions: false,
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));
        let h = PermissionsHandler;
        let bogus = "bogus-mode-xyz";
        let expected_msg =
            archon_tools::validation::validate_permission_mode(bogus)
                .expect_err("'bogus-mode-xyz' must NOT be a valid permission mode");

        let res = h.execute(&mut ctx, &[bogus.to_string()]);
        assert!(
            res.is_ok(),
            "PermissionsHandler::execute(invalid) must return Ok(()), got: {res:?}"
        );

        // 1. NO effect stashed.
        assert!(
            ctx.pending_effect.is_none(),
            "invalid-arg branch must NOT stash a CommandEffect; got: {:?}",
            ctx.pending_effect
        );

        // 2. Exactly one Error event with byte-identical payload.
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
                    "invalid-arg branch Error must match validate_permission_mode \
                     output byte-for-byte (pass-through)"
                );
            }
            other => panic!(
                "invalid-arg branch must emit TuiEvent::Error, got: {other:?}"
            ),
        }
        // 3. NO TextDelta.
        let has_delta = events.iter().any(|e| matches!(e, TuiEvent::TextDelta(_)));
        assert!(
            !has_delta,
            "invalid-arg branch must emit NO TuiEvent::TextDelta; got: {events:?}"
        );
    }

    /// Missing snapshot on the empty-arg branch must surface as a
    /// loud Err (defensive — mirrors ModelHandler/EffortHandler
    /// stance against silent drift).
    #[test]
    fn permissions_handler_execute_no_args_without_snapshot_returns_err() {
        let (mut ctx, _rx) = make_ctx(None);
        let h = PermissionsHandler;
        let result = h.execute(&mut ctx, &[]);
        assert!(
            result.is_err(),
            "PermissionsHandler::execute must return Err when \
             permissions_snapshot is None on the empty-arg branch \
             (defensive: builder bug should surface loudly)"
        );
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("permissions_snapshot")
                || err_msg.contains("build_command_context"),
            "error must describe the missing snapshot, got: {err_msg}"
        );
    }

    // ── Gate 5 dispatcher-integration tests ──────────────────────
    //
    // Prove end-to-end routing via the real `Dispatcher` +
    // `default_registry()` harness: (a) slash input hits
    // PermissionsHandler (not the deleted slash.rs arm, not some other
    // handler), (b) byte-identity of the shipped TextDelta strings
    // survives the full dispatch chain, (c) the HYBRID pattern's two
    // slots (permissions_snapshot READ, pending_effect ASYNC-WRITE)
    // wire correctly through the dispatcher.
    //
    // Reference template: src/command/effort.rs dispatcher tests
    // (B11-EFFORT Gate 5). Zero mocks: Arc<Registry> from
    // `default_registry()` + `Dispatcher::new` exactly as the live
    // harness builds them in session.rs.

    #[test]
    fn dispatcher_routes_slash_permissions_to_handler_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);

        // Bare "/permissions" → READ branch → TextDelta with snapshot
        // mode. The handler reads `permissions_snapshot` (populated
        // here inline — in production the builder fills it before
        // dispatch). Use "default" as the harness default.
        let snap = PermissionsSnapshot {
            current_mode: "default".to_string(),
            allow_bypass_permissions: false,
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));

        let result = dispatcher.dispatch(&mut ctx, "/permissions");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/permissions\") must return Ok; got: {result:?}"
        );

        // 1. NO pending_effect (empty-arg branch is READ-only).
        assert!(
            ctx.pending_effect.is_none(),
            "end-to-end bare `/permissions` must NOT stash a \
             CommandEffect; got: {:?}",
            ctx.pending_effect
        );

        // 2. Exactly one TextDelta whose payload is byte-identical to
        //    the shipped format!() output for the snapshot mode.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "end-to-end bare `/permissions` must emit exactly one \
             event; got: {events:?}"
        );
        let expected = format!(
            "\nCurrent permission mode: {}\n\
             Usage: /permissions <mode>\n\
             Modes: default, acceptEdits, plan, auto, dontAsk, \
             bypassPermissions\n\
             Legacy aliases: ask -> default, yolo -> \
             bypassPermissions\n",
            "default"
        );
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, &expected,
                    "end-to-end bare `/permissions` TextDelta must \
                     match shipped format! byte-for-byte"
                );
            }
            other => panic!(
                "end-to-end bare `/permissions` must emit \
                 TuiEvent::TextDelta, got: {other:?}"
            ),
        }

        // 3. NO Error event.
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            !has_error,
            "end-to-end bare `/permissions` must emit NO \
             TuiEvent::Error; got: {events:?}"
        );
    }

    #[test]
    fn dispatcher_routes_slash_permissions_with_plan_arg_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use crate::command::registry::CommandEffect;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        // WRITE branch still needs snapshot for the bypass-allow
        // guard (handler defensively re-reads snapshot on WRITE).
        // Production builder always populates when primary resolves
        // to /permissions, so match that contract here.
        let snap = PermissionsSnapshot {
            current_mode: "default".to_string(),
            allow_bypass_permissions: false,
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));

        // "/permissions plan" → WRITE branch → effect stash +
        // TextDelta confirmation.
        let result = dispatcher.dispatch(&mut ctx, "/permissions plan");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/permissions plan\") must return \
             Ok; got: {result:?}"
        );

        // 1. pending_effect MUST be Some(SetPermissionMode("plan")).
        match ctx.pending_effect.as_ref() {
            Some(CommandEffect::SetPermissionMode(resolved)) => {
                assert_eq!(
                    resolved, "plan",
                    "SetPermissionMode must carry the resolved mode \
                     string from the dispatched arg"
                );
            }
            other => panic!(
                "expected Some(CommandEffect::SetPermissionMode(\"plan\")), \
                 got: {other:?}"
            ),
        }

        // 2. Exactly one TextDelta whose payload is byte-identical to
        //    the shipped success format!() output.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "end-to-end `/permissions plan` must emit exactly one \
             event; got: {events:?}"
        );
        let expected = "\nPermission mode set to plan.\n".to_string();
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, &expected,
                    "end-to-end `/permissions plan` TextDelta must \
                     match shipped format! byte-for-byte"
                );
            }
            other => panic!(
                "end-to-end `/permissions plan` must emit \
                 TuiEvent::TextDelta, got: {other:?}"
            ),
        }

        // 3. NO Error event.
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            !has_error,
            "end-to-end `/permissions plan` must emit NO \
             TuiEvent::Error; got: {events:?}"
        );
    }
}
