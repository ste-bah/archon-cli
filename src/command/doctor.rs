//! TASK-AGS-POST-6-BODIES-B15-DOCTOR: /doctor slash-command handler
//! (body-migrate, SNAPSHOT-DELEGATE pattern — READ-only).
//!
//! Reference: shipped async delegate `handle_doctor_command` at
//! `src/command/doctor.rs` (pre-migration) + legacy match arm at
//! `src/command/slash.rs:230-234`.
//! Source:   shipped `declare_handler!(DoctorHandler, "Run environment
//!           health checks")` stub at `src/command/registry.rs:1095`
//!           (no aliases).
//!
//! # R1 PATTERN-CONFIRM (SNAPSHOT-DELEGATE chosen)
//!
//! Unlike the shipped bodies migrated in B01..B14 (which lived inside
//! the legacy `handle_slash_command` match), `/doctor` already lives in
//! this file as a dedicated `pub async fn handle_doctor_command`
//! extracted from `main.rs` by a prior refactor. The shipped delegate
//! composes a multi-line diagnostic string from SEVEN inputs on
//! `SlashCommandContext`:
//!
//! 1. **Sync read** `ctx.auth_label` — static `String` (no lock).
//! 2. **Async READ** `ctx.mcp_manager.get_server_states().await` — the
//!    first of TWO `.await` points. Returns `HashMap<String, ServerState>`.
//! 3. **Sync read** `ctx.memory.memory_count()` — `MemoryTrait::memory_count`
//!    is plain `fn` (AGS-817 precedent), no await.
//! 4. **Sync read** `ctx.config_path.exists()` + `.display()` — filesystem
//!    probe, no lock.
//! 5. **Sync read** `dirs::data_dir()` + `ckpt_path.exists()` — filesystem
//!    probe, no lock.
//! 6. **Async READ** `ctx.model_override_shared.lock().await` — the second
//!    `.await` point. Pick override vs `ctx.default_model` fallback.
//! 7. **Sync read** `ctx.env_vars` → `archon_core::env_vars::format_doctor_env_vars`
//!    — plain `fn`, no lock.
//! 8. **Single sync emission** of `TuiEvent::TextDelta(out)` — shipped
//!    uses `.send(..).await` but `try_send` preserves identity for a
//!    single-receiver fast path (post-migration ordering verified via
//!    tests).
//!
//! The final string is a single TextDelta. Because the ONLY async points
//! are (2) the MCP state read and (6) the model-override lock, this is a
//! textbook SNAPSHOT migration (AGS-807 / AGS-808 / B08 / B11 / B12 / B14
//! precedent), with the additional twist that the existing `async fn`
//! delegate is DECOMPOSED rather than deleted: the async composition
//! becomes `pub async fn build_doctor_text(&SlashCommandContext) -> String`
//! and the shipped `handle_doctor_command` delegate thins down to a
//! `build_doctor_text + tui_tx.send().await` pair. The builder awaits
//! the composed text and stores a single owned `String` on the new
//! `CommandContext::doctor_snapshot` field; the sync `DoctorHandler`
//! consumes the snapshot and emits via `try_send`.
//!
//! * **READ side → SNAPSHOT pattern**. A new
//!   `doctor_snapshot: Option<DoctorSnapshot>` field on `CommandContext`
//!   is populated by `build_command_context` ONLY when the primary
//!   resolves to `/doctor`. The builder awaits `build_doctor_text` and
//!   clones the composed String into the snapshot so the sync handler
//!   can emit without locking.
//!
//! # R2 PRIMARY-ALREADY-REGISTERED
//!
//! `doctor` is already a primary in the default registry via the
//! `declare_handler!(DoctorHandler, "Run environment health checks")`
//! stub at registry.rs:1095 (no aliases). This ticket is a
//! body-migrate, NOT a gap-fix: primary count is UNCHANGED. The stub
//! is REMOVED in favour of the real type defined in this file,
//! imported at the top of registry.rs, and kept at the existing
//! `insert_primary("doctor", Arc::new(DoctorHandler::new()))` site.
//!
//! # R3 ALIASES (zero — preserved from shipped)
//!
//! The shipped stub used the two-arg `declare_handler!` form (no
//! aliases slice). Zero aliases preserved. Pinned by test
//! `doctor_handler_aliases_are_empty`.
//!
//! # R4 ARG SEMANTICS
//!
//! The shipped arm matched `/doctor` literally — no args were consumed.
//! Post-migration, the handler's `args: &[String]` is IGNORED in every
//! branch. Any trailing tokens after `/doctor` simply route here and
//! are silently discarded — byte-identical to shipped behaviour (the
//! `"/doctor" =>` arm did not even parse a strip_prefix remainder).
//!
//! # R5 DELEGATE COMPOSITION (no trait extension)
//!
//! Unlike B14 /copy (which introduced an internal `ClipboardRunner`
//! trait to make subprocess work testable), `/doctor` has no
//! subprocess work — every line of the composition is a direct read
//! from `SlashCommandContext`. The R5 slot is UNUSED for this ticket;
//! testability is achieved by asserting on the exact emitted String
//! rather than probing per-input branches. Unit tests drive the
//! handler with a synthetic `DoctorSnapshot { text: "..." }` rather
//! than stubbing seven fields of `SlashCommandContext`.
//!
//! # R6 EMISSION ORDER (unchanged vs shipped)
//!
//! Shipped order:
//!
//! ```ignore
//! // at the tail of handle_doctor_command:
//! let _ = tui_tx.send(TuiEvent::TextDelta(out)).await;
//! ```
//!
//! Post-migration uses `try_send` instead of `.send(..).await`. Every
//! production deployment has a drained mpsc receiver (TUI event loop)
//! so the fast path is identical; `try_send` never blocks and returns
//! `Err` only if the channel is full or closed, which is never true at
//! the /doctor dispatch site. The dispatcher contract (handler must
//! return `anyhow::Result<()>` synchronously) forbids `.await`, so
//! `try_send` is the only option — matches every prior B-series
//! migration.
//!
//! # R7 TEMPORARY DOUBLE-FIRE NOTE (Gates 1-4 scope)
//!
//! For Gates 1-4 of this ticket the legacy match arm at
//! `src/command/slash.rs:230-234` is LEFT INTACT. Because
//! `dispatcher.dispatch` fires the handler BEFORE the recognized-
//! command short-circuit allows fall-through into the match, `/doctor`
//! will fire `DoctorHandler` AND the legacy arm on every input. Gate 5
//! (live-smoke + legacy-arm deletion) removes the double fire in
//! production. Mirrors every prior B-series migration.
//!
//! The key implication for Gates 1-4 testing: the handler SHOULD be
//! exercised only through the unit-test suite (`cargo test
//! command::doctor`) — the legacy `handle_doctor_command` delegate is
//! still in scope for production use while the double-fire transition
//! window is open. Gate 5 deletes both the legacy arm AND thins
//! `handle_doctor_command` (or deletes it entirely if no other caller
//! remains).

use std::path::PathBuf;

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};
use crate::slash_context::SlashCommandContext;

// ---------------------------------------------------------------------------
// Shipped delegate (thin wrapper around build_doctor_text)
// ---------------------------------------------------------------------------

/// Handle `/doctor` — diagnostic status display.
///
// TASK-AGS-POST-6-BODIES-B15-DOCTOR Gate 5: the shipped
// `pub async fn handle_doctor_command(tui_tx, ctx)` async delegate
// that composed the diagnostic text and emitted it via
// `tui_tx.send(TuiEvent::TextDelta(out)).await` has been DELETED.
// It was the legacy slash.rs:230 call site and had no other callers
// (verified by grep at Gate 5). All composition now lives in
// [`build_doctor_text`] (below) and emission is driven synchronously
// from [`DoctorHandler::execute`] consuming a pre-built
// [`DoctorSnapshot`]. The byte-identity of the composed text is
// preserved — the pre-migration function body was extracted verbatim
// into [`build_doctor_text`].

/// Compose the multi-line `/doctor` diagnostic text. Returns the exact
/// String previously sent as `TuiEvent::TextDelta` by the shipped
/// delegate — byte-identical output, same awaits in the same positions.
///
/// Extracted from `handle_doctor_command` so:
///
/// 1. The legacy slash.rs delegate and the new snapshot builder can
///    share one source of truth for the diagnostic text.
/// 2. The new [`build_doctor_snapshot`] can await this helper inside
///    `build_command_context` where `.await` is allowed, then hand
///    the owned String off to the sync [`DoctorHandler`].
///
/// # Awaits
///
/// Two `.await` points preserved from shipped:
///
/// * `ctx.mcp_manager.get_server_states().await` — per-server state
///   enumeration.
/// * `ctx.model_override_shared.lock().await` — model override lock.
///
/// Every other read is sync (`memory_count`, filesystem probes,
/// `format_doctor_env_vars`).
pub async fn build_doctor_text(ctx: &SlashCommandContext) -> String {
    use archon_core::env_vars;

    let mut out = String::from("\nArchon diagnostics:\n");

    // Auth
    out.push_str(&format!("  Auth: authenticated ({})\n", ctx.auth_label));

    // MCP servers
    let states = ctx.mcp_manager.get_server_states().await;
    if states.is_empty() {
        out.push_str("  MCP servers: none configured\n");
    } else {
        out.push_str(&format!("  MCP servers: {} configured\n", states.len()));
        for (name, state) in &states {
            out.push_str(&format!("    {name}: {state}\n"));
        }
    }

    // Memory graph
    match ctx.memory.memory_count() {
        Ok(count) => out.push_str(&format!("  Memory graph: open ({count} memories)\n")),
        Err(e) => out.push_str(&format!("  Memory graph: error ({e})\n")),
    }

    // Config
    let config_valid = ctx.config_path.exists();
    out.push_str(&format!(
        "  Config: {} ({})\n",
        ctx.config_path.display(),
        if config_valid { "valid" } else { "not found" },
    ));

    // Checkpoint store
    let ckpt_path = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("archon")
        .join("checkpoints.db");
    let ckpt_status = if ckpt_path.exists() { "open" } else { "closed" };
    out.push_str(&format!("  Checkpoint store: {ckpt_status}\n"));

    // Model
    let current_model = {
        let ov = ctx.model_override_shared.lock().await;
        if ov.is_empty() {
            ctx.default_model.clone()
        } else {
            ov.clone()
        }
    };
    out.push_str(&format!("  Model: {current_model}\n"));

    // Environment variables
    out.push_str(&env_vars::format_doctor_env_vars(&ctx.env_vars));

    out
}

// ---------------------------------------------------------------------------
// Snapshot
// ---------------------------------------------------------------------------

/// Owned snapshot of the fully-composed `/doctor` diagnostic text
/// consumed by the sync handler.
///
/// Built at the dispatch site by `build_command_context` (where
/// `.await` is allowed) via [`build_doctor_snapshot`] and threaded
/// through [`CommandContext`] so the sync [`DoctorHandler`] can emit
/// the TextDelta without holding any lock.
///
/// Carries a single owned `String` because the shipped delegate emits
/// the composed text in exactly ONE way: a single
/// `TuiEvent::TextDelta(out)` at the tail. No partial observations on
/// the content — the snapshot is a straight clone of the full output.
#[derive(Debug, Clone)]
pub(crate) struct DoctorSnapshot {
    /// Fully composed multi-line diagnostic text, byte-identical to
    /// the `out` local in the pre-migration delegate at
    /// `handle_doctor_command`. Zero-length strings are legitimate
    /// (tests exercise this branch) — the handler emits them verbatim.
    pub(crate) text: String,
}

/// Build a [`DoctorSnapshot`] by awaiting [`build_doctor_text`] in the
/// SAME position as the shipped READ path at `handle_doctor_command`.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/doctor`. All other commands leave
/// `doctor_snapshot = None` to avoid unnecessary lock traffic on
/// `mcp_manager` / `model_override_shared`.
pub(crate) async fn build_doctor_snapshot(
    slash_ctx: &SlashCommandContext,
) -> DoctorSnapshot {
    DoctorSnapshot {
        text: build_doctor_text(slash_ctx).await,
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// Real `/doctor` handler — consumes a pre-built [`DoctorSnapshot`]
/// from [`CommandContext`] and emits the composed text as a single
/// `TuiEvent::TextDelta` via `try_send`.
///
/// # Branch matrix
///
/// * `doctor_snapshot == None` → `anyhow::Err` (wiring regression —
///   `build_command_context` bypassed or alias map drifted). Mirrors
///   B14 `CopyHandler` defensive stance.
/// * `doctor_snapshot == Some(snap)` → single
///   `TuiEvent::TextDelta(snap.text.clone())` via `try_send`. Empty
///   strings are legitimate and emit zero-length TextDelta events
///   (matches shipped — `TuiEvent::TextDelta(String::new())` round-trips
///   through the TUI event loop cleanly).
pub(crate) struct DoctorHandler;

impl DoctorHandler {
    /// Default production constructor — zero-sized struct, no state.
    pub(crate) fn new() -> Self {
        Self
    }
}

impl Default for DoctorHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandHandler for DoctorHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        _args: &[String],
    ) -> anyhow::Result<()> {
        // R4: args are IGNORED. The shipped delegate took `ctx` but no
        // args; the legacy match arm at slash.rs:230-234 matched
        // `/doctor` literally with no strip_prefix — trailing tokens
        // were silently discarded. Preserved here via the `_args`
        // parameter rename.

        // Consume the pre-built snapshot populated by
        // `build_command_context` when the primary resolved to
        // `/doctor`. A `None` here indicates a wiring regression
        // (builder bypassed or alias map drifted); surface it as a
        // loud `Err` rather than a user-facing message (mirrors B14
        // CopyHandler defensive stance).
        let snap = ctx.doctor_snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "DoctorHandler invoked without doctor_snapshot populated — \
                 build_command_context wiring regression"
            )
        })?;

        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(snap.text.clone()));
        Ok(())
    }

    fn description(&self) -> &str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:1095 (shipped-wins drift-reconcile).
        "Run environment health checks"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R3: zero aliases shipped → zero aliases preserved. Pinned
        // by test `doctor_handler_aliases_are_empty`.
        &[]
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    // ---- make_ctx -----------------------------------------------------

    /// Build a `CommandContext` with a freshly-created channel and an
    /// optional [`DoctorSnapshot`]. All other optional fields stay
    /// `None`. Mirrors the make_ctx fixtures in copy.rs / permissions.rs /
    /// effort.rs / add_dir.rs.
    fn make_ctx(
        snapshot: Option<DoctorSnapshot>,
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
                permissions_snapshot: None,
                copy_snapshot: None,
                doctor_snapshot: snapshot,
                usage_snapshot: None,
                config_path: None,
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    /// Drain `rx` non-blockingly into a Vec — matches the `drain`
    /// helper in copy.rs / permissions.rs / effort.rs test modules.
    fn drain(rx: &mut mpsc::Receiver<TuiEvent>) -> Vec<TuiEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    // ---- R1: description byte-identity + aliases empty ----------------

    #[test]
    fn doctor_handler_description_byte_identical_to_shipped() {
        let h = DoctorHandler::new();
        assert_eq!(
            h.description(),
            "Run environment health checks",
            "DoctorHandler description must match the shipped \
             declare_handler! stub at registry.rs:1095 byte-for-byte \
             (shipped-wins drift-reconcile)"
        );
    }

    #[test]
    fn doctor_handler_aliases_are_empty() {
        let h = DoctorHandler::new();
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "DoctorHandler aliases must be empty to match the shipped \
             declare_handler! stub (two-arg form, no aliases slice)"
        );
    }

    // ---- R2: snapshot-missing Err branch ------------------------------

    #[test]
    fn doctor_handler_execute_without_snapshot_returns_err() {
        let (mut ctx, _rx) = make_ctx(None);
        let h = DoctorHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_err(),
            "DoctorHandler::execute must return Err when doctor_snapshot \
             is None (builder contract violation), got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.to_lowercase().contains("doctor_snapshot"),
            "Err message must mention 'doctor_snapshot' for operator \
             traceability, got: {msg}"
        );
        assert!(
            msg.contains("wiring") || msg.contains("builder"),
            "Err message must mention 'wiring' or 'builder' to locate \
             the fix site, got: {msg}"
        );
    }

    // ---- R6: emission byte-identity (happy path) ----------------------

    #[test]
    fn doctor_handler_execute_emits_textdelta_byte_exact() {
        // Synthetic snapshot — the handler's only job is to emit the
        // snapshot's text verbatim, so we drive it with a deterministic
        // sentinel string rather than standing up a full
        // SlashCommandContext to exercise build_doctor_text.
        let snap = DoctorSnapshot {
            text: "synthetic-diag".to_string(),
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));
        let h = DoctorHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "happy path must return Ok (emission via TuiEvent), got: {res:?}"
        );

        // Exactly one TextDelta event with byte-identical content.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "happy path must emit exactly one event; got: {events:?}"
        );
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "synthetic-diag",
                    "TextDelta must carry the snapshot's text \
                     byte-for-byte"
                );
            }
            other => panic!(
                "happy path must emit TuiEvent::TextDelta, got: {other:?}"
            ),
        }
    }

    // ---- Edge case: zero-length snapshot still emits ------------------

    #[test]
    fn doctor_handler_execute_empty_text_still_emits_textdelta() {
        // Zero-length text is a legitimate snapshot outcome — the
        // handler MUST emit a TextDelta("") rather than short-circuit
        // with Err or silently skipping. Mirrors the "every snapshot
        // is authoritative" invariant from B14 CopyHandler.
        let snap = DoctorSnapshot {
            text: String::new(),
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));
        let h = DoctorHandler::new();
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "empty-text snapshot must return Ok (emission via \
             TuiEvent), got: {res:?}"
        );

        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "empty-text snapshot must still emit exactly one event; \
             got: {events:?}"
        );
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "",
                    "empty-text snapshot must emit TextDelta(\"\") \
                     byte-for-byte"
                );
            }
            other => panic!(
                "empty-text snapshot must emit TuiEvent::TextDelta, \
                 got: {other:?}"
            ),
        }
    }

    // ---- Gate 5: dispatcher-integration end-to-end --------------------
    //
    // Route `/doctor` through the real Dispatcher + Registry + handler
    // stack to pin post-arm-delete wiring. Builds a NARROW
    // `RegistryBuilder` (not `default_registry`) to keep the test
    // scope tight — only `/doctor` is registered, so any routing
    // regression surfaces immediately rather than getting masked by
    // unrelated handlers.

    #[test]
    fn dispatcher_routes_slash_doctor_with_snapshot_emits_textdelta() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::RegistryBuilder;
        use std::sync::Arc;

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("doctor", Arc::new(DoctorHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let snap = DoctorSnapshot {
            text: "dispatcher-integration diag".to_string(),
        };
        let (mut ctx, mut rx) = make_ctx(Some(snap));

        let res = dispatcher.dispatch(&mut ctx, "/doctor");
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

        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "exactly one event must be emitted through the dispatcher, \
             got: {events:?}"
        );
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, "dispatcher-integration diag",
                    "TextDelta must carry the snapshot's text \
                     byte-for-byte through Dispatcher → Registry → \
                     DoctorHandler::execute"
                );
            }
            other => panic!(
                "dispatcher must route /doctor to a TextDelta emission, \
                 got: {other:?}"
            ),
        }
    }

    #[test]
    fn dispatcher_routes_slash_doctor_without_snapshot_returns_err() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::RegistryBuilder;
        use std::sync::Arc;

        let mut builder = RegistryBuilder::new();
        builder.insert_primary("doctor", Arc::new(DoctorHandler::new()));
        let registry = Arc::new(builder.build());
        let dispatcher = Dispatcher::new(registry);

        let (mut ctx, mut rx) = make_ctx(None);

        let res = dispatcher.dispatch(&mut ctx, "/doctor");
        assert!(
            res.is_err(),
            "dispatcher.dispatch must propagate DoctorHandler's Err \
             when doctor_snapshot is None (builder contract violation), \
             got: {res:?}"
        );
        let msg = format!("{:#}", res.unwrap_err());
        assert!(
            msg.to_lowercase().contains("doctor_snapshot"),
            "dispatcher-propagated Err must mention 'doctor_snapshot', \
             got: {msg}"
        );

        // No TextDelta emitted on the Err path — the handler short-
        // circuits before try_send.
        let events = drain(&mut rx);
        assert!(
            events.is_empty(),
            "Err path must not emit any TuiEvent, got: {events:?}"
        );
    }
}
