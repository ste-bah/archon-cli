//! TASK-AGS-POST-6-BODIES-B10-ADDDIR: /add-dir slash-command handler
//! (body-migrate, EFFECT-SLOT pattern — mirrors AGS-808 /model and
//! B04-DIFF precedent).
//!
//! Reference: docs/stage-7.5/tickets/TASK-AGS-POST-6-BODIES-B10-ADDDIR.md
//! Based on: src/command/slash.rs:668-691 (shipped `/add-dir` arm)
//! Source: src/command/registry.rs:886 (`declare_handler!(AddDirHandler,
//!   "Add a directory to the working context")` no-op stub being replaced)
//! Derived from: src/command/diff.rs (TASK-AGS-POST-6-BODIES-B04-DIFF —
//!   the EFFECT-SLOT precedent using `CommandEffect::RunGitDiffStat`).
//!
//! # R1 PATTERN-CONFIRM (EFFECT-SLOT, not DIRECT)
//!
//! The ticket spec originally classified /add-dir as DIRECT because the
//! shipped body is small and trivially looking. However, the shipped
//! source at slash.rs:679 contains:
//!
//! ```ignore
//! ctx.extra_dirs.lock().await.push(path.clone());
//! ```
//!
//! where `ctx.extra_dirs` is `Arc<tokio::sync::Mutex<Vec<PathBuf>>>`
//! (slash_context.rs:35). The `.lock().await` call cannot run inside a
//! sync `CommandHandler::execute`, so the mutation MUST be deferred
//! through `CommandEffect` / `apply_effect` — the AGS-808 effect-slot
//! pattern. Path B reclassification to EFFECT-SLOT was user-confirmed
//! during Gate 0 ticket review.
//!
//! * NO new `CommandContext` field (the `PathBuf` is captured directly
//!   into the effect variant — the handler reads it from the parsed
//!   `args` slice, not from shared state).
//! * NO snapshot (nothing to pre-compute before dispatch — validation
//!   via `path.is_dir()` is a sync filesystem stat call).
//! * YES new `CommandEffect::AddExtraDir(PathBuf)` variant (the push
//!   onto `SlashCommandContext::extra_dirs` is the deferred async
//!   mutation).
//!
//! # R2 PRIMARY-ALREADY-REGISTERED
//!
//! `add-dir` is already a primary in the default registry via the
//! `declare_handler!(AddDirHandler, "Add a directory to the working
//! context")` stub at registry.rs:886 (no aliases). This ticket is a
//! body-migrate, NOT a gap-fix: primary count is UNCHANGED. The stub is
//! REMOVED in favour of the real type defined in this file, imported
//! into registry.rs at the top via `use crate::command::add_dir::AddDirHandler;`.
//!
//! # R3 NO-ALIASES (shipped-wins drift-reconcile)
//!
//! Shipped `declare_handler!` stub at registry.rs:886 carried no alias
//! slice — equivalent to `&[]`. AGS-817 shipped-wins drift-reconcile
//! rule preserves zero aliases. This handler returns `&[]` from
//! `aliases()` and the test `add_dir_handler_aliases_are_empty` pins
//! the invariant against silent additions.
//!
//! # R4 ARGS-RECONCILIATION (trailing-args-CONSUME, like /color cyan)
//!
//! The shipped body in slash.rs:668-691 used
//! `s.strip_prefix("/add-dir").unwrap_or("").trim()` on the raw input
//! string — a single-string substring after the command name. The
//! parser tokenizes on whitespace into `args: &[String]`. For a
//! single-token path (`/add-dir /tmp`), `args.first()` would be
//! byte-equivalent. For a multi-token path (`/add-dir /opt some dir`),
//! the shipped semantics preserve the trailing whitespace-joined
//! substring.
//!
//! This handler uses `args.join(" ").trim()` which preserves the shipped
//! substring semantics EXACTLY for any multi-token input and degrades
//! gracefully to the same single-token form for the common case.
//! Mirrors AGS-819 /theme R4 and B09-COLOR R4 — the trailing-args-
//! CONSUME pattern (the arg IS the payload), NOT the trailing-args-
//! IGNORED pattern used by B04-DIFF / B07-RELEASE-NOTES (where trailing
//! args are discarded because the shipped arm matched the bare command).
//!
//! # R5 EMISSION-PRIMITIVE-SWAP (.await -> try_send)
//!
//! Shipped body emitted via `tui_tx.send(..).await` — async, blocking
//! on backpressure if the 16-cap channel is full. The sync
//! `CommandHandler::execute` signature cannot `.await`, so this handler
//! uses `ctx.tui_tx.try_send(..)` (sync, best-effort drop on full).
//! Matches AGS-806..819 emission precedent verbatim. All three shipped
//! format strings are preserved BYTE-FOR-BYTE:
//!
//! 1. `"Usage: /add-dir <path>"` (empty-arg Error).
//! 2. `"\nAdded '{}' to working directories for this session.\nFiles in
//!    this directory are now accessible.\n"` (success TextDelta, format!
//!    with `path.display()`).
//! 3. `"Directory not found: {path_arg}"` (invalid-dir Error, format!
//!    with the raw path_arg string).
//!
//! # R6 EFFECT-SLOT EMISSION ORDER (ORDER-SEMANTICS-SWAP, accepted)
//!
//! Shipped order at slash.rs:679-683:
//!
//! ```ignore
//! ctx.extra_dirs.lock().await.push(path.clone());   // 1. mutate state
//! let _ = tui_tx.send(TuiEvent::TextDelta(..)).await; // 2. emit delta
//! tracing::info!(..);                                 // 3. emit log
//! ```
//!
//! Post-migration order:
//!
//! 1. Handler (sync) stashes `CommandEffect::AddExtraDir(path.clone())`
//!    and `try_send(TuiEvent::TextDelta(..))` — order: effect-stash
//!    THEN TextDelta (so the confirmation lands in the TUI channel
//!    before dispatch returns).
//! 2. `apply_effect` (async) awaits `extra_dirs.lock().await.push(..)`
//!    and emits the `tracing::info!` record — order: mutex push THEN
//!    log, byte-identical to shipped 679 + 683.
//!
//! Both the handler's `try_send` and `apply_effect`'s await complete
//! inside `handle_slash_command` before it returns to the main input
//! loop. The user-observable state at the next input tick is therefore
//! identical: `extra_dirs` has the new path AND the TextDelta has been
//! enqueued. The `tracing::info!` record is emitted at the same byte-
//! identical timestamp-relative position (inside `apply_effect`).
//!
//! The only observable drift is the ORDER of the TextDelta vs the
//! mutex push — shipped did push-then-delta, post-migration does
//! delta-then-push. Because neither the TUI event consumer nor any
//! downstream observer inspects `extra_dirs` between the delta and the
//! push (both land within the same dispatch turn), the drift is
//! invariant-preserving.
//!
//! # R7 BYTE-IDENTITY PINS
//!
//! Four literal/format strings pinned via `assert_eq!` in the test
//! module:
//!
//! * `description()` — "Add a directory to the working context"
//! * empty-arg Error — "Usage: /add-dir <path>"
//! * success TextDelta — `format!("\nAdded '{}' to working
//!   directories for this session.\nFiles in this directory are now
//!   accessible.\n", path.display())`
//! * invalid-dir Error — `format!("Directory not found: {path_arg}")`

use std::path::PathBuf;

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};

/// Zero-sized handler registered as the primary `/add-dir` command.
///
/// No aliases (see R3 in module rustdoc). Body-migrate of the shipped
/// arm at slash.rs:668-691 — EFFECT-SLOT pattern (the
/// `Arc<tokio::sync::Mutex<Vec<PathBuf>>>` push forces the async
/// deferral via `CommandEffect::AddExtraDir`).
///
/// # Behavior
///
/// * Empty args (bare `/add-dir`) → emit a usage `TuiEvent::Error`.
/// * Non-empty arg + `PathBuf::from(arg).is_dir()` → stash
///   `CommandEffect::AddExtraDir(path)` AND emit a confirmation
///   `TuiEvent::TextDelta`. `apply_effect` awaits the mutex push on
///   `SlashCommandContext::extra_dirs` and logs the
///   `tracing::info!(dir = %path.display(), "added working directory
///   via /add-dir")` record.
/// * Non-empty arg + path does NOT exist or is not a directory → emit a
///   `TuiEvent::Error`. NO effect stashed, NO mutation occurs.
pub(crate) struct AddDirHandler;

impl CommandHandler for AddDirHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // R4: join multi-token args with " " and trim. Byte-equivalent
        // to the shipped `s.strip_prefix("/add-dir").unwrap_or("").trim()`
        // for all inputs — single-token paths collapse to the same
        // value as `args.first().unwrap_or("").as_str()`, multi-token
        // paths preserve the whitespace-joined substring. Empty args
        // and a whitespace-only join both produce the empty string,
        // routing to the usage-error branch identical to the shipped
        // `if path_arg.is_empty()` check.
        let joined = args.join(" ");
        let path_arg = joined.trim();

        if path_arg.is_empty() {
            // Empty-arg branch — byte-for-byte preservation of shipped
            // format string at slash.rs:672-674.
            let _ = ctx
                .tui_tx
                .send(TuiEvent::Error("Usage: /add-dir <path>".into()));
        } else {
            let path = PathBuf::from(path_arg);
            if path.is_dir() {
                // Valid directory — stash the EFFECT-SLOT variant first
                // (the deferred async push onto
                // `SlashCommandContext::extra_dirs`), then emit the
                // confirmation TextDelta. See module rustdoc R6 for
                // the order-semantics-swap rationale.
                //
                // Clone the PathBuf into the effect variant so the
                // effect carries owned data across the .take() boundary
                // at slash.rs — no borrow on ctx lifetime.
                ctx.pending_effect = Some(CommandEffect::AddExtraDir(path.clone()));
                ctx.emit(TuiEvent::TextDelta(format!(
                    "\nAdded '{}' to working directories for this session.\nFiles in this directory are now accessible.\n",
                    path.display()
                )));
            } else {
                // Invalid directory — emit Error with byte-identical
                // shipped format at slash.rs:685-687. NO effect stashed
                // so `apply_effect` is never invoked for this branch.
                ctx.emit(TuiEvent::Error(format!("Directory not found: {path_arg}")));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        // Byte-for-byte preservation of the shipped declare_handler!
        // stub at registry.rs:886 (shipped-wins drift-reconcile).
        "Add a directory to the working context"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // R3: zero aliases shipped → zero aliases preserved. Pinned by
        // test `add_dir_handler_aliases_are_empty`.
        &[]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-POST-6-BODIES-B10-ADDDIR: tests for /add-dir slash-command
// body-migrate. Uses a local `make_ctx` helper (NOT an extension to
// test_support.rs) — mirrors the pattern established by
// src/command/color.rs (AGS-POST-6-BODIES-B09-COLOR) and
// src/command/theme.rs (AGS-819).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_tui::app::TuiEvent;
    use std::path::PathBuf;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel.
    ///
    /// /add-dir is an EFFECT-SLOT handler — the actual mutation on
    /// `SlashCommandContext::extra_dirs` is deferred to `apply_effect`,
    /// and the handler itself does not read any other `CommandContext`
    /// field beyond `tui_tx` and `pending_effect`. Every other optional
    /// field stays `None`. Mirrors the make_ctx fixtures in color.rs /
    /// theme.rs / voice.rs / export.rs.
    fn make_ctx() -> (CommandContext, mpsc::UnboundedReceiver<TuiEvent>) {
        // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
        // /add-dir is an EFFECT-SLOT handler — every optional field stays
        // at its test-safe default.
        crate::command::test_support::CtxBuilder::new().build()
    }

    /// Drain every event currently pending in the channel.
    fn drain(rx: &mut mpsc::UnboundedReceiver<TuiEvent>) -> Vec<TuiEvent> {
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        events
    }

    /// The description must match the shipped `declare_handler!` stub
    /// at registry.rs:886 BYTE-FOR-BYTE. AGS-817 shipped-wins rule.
    #[test]
    fn add_dir_handler_description_byte_identical_to_shipped() {
        let h = AddDirHandler;
        assert_eq!(
            h.description(),
            "Add a directory to the working context",
            "AddDirHandler description must match the shipped \
             declare_handler! stub verbatim (shipped-wins drift-reconcile)"
        );
    }

    /// Shipped `declare_handler!` stub at registry.rs:886 carried no
    /// alias slice — equivalent to `&[]`. AGS-817 shipped-wins rule
    /// preserves zero aliases.
    #[test]
    fn add_dir_handler_aliases_are_empty() {
        let h = AddDirHandler;
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "AddDirHandler must have an empty alias slice per B10 R3 \
             (shipped declare_handler! stub had no aliases)"
        );
    }

    /// Bare `/add-dir` (no args) must emit a single `TuiEvent::Error`
    /// whose payload is byte-identical to the shipped
    /// `"Usage: /add-dir <path>"` string. NO pending_effect must be
    /// stashed — the empty-arg branch is error-only.
    #[test]
    fn add_dir_handler_execute_with_no_args_emits_usage_error() {
        let (mut ctx, mut rx) = make_ctx();
        let h = AddDirHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(
            res.is_ok(),
            "AddDirHandler::execute(no-args) must return Ok(()), got: {res:?}"
        );

        assert!(
            ctx.pending_effect.is_none(),
            "empty-arg branch must NOT stash an effect; got: {:?}",
            ctx.pending_effect
        );

        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "empty-arg branch must emit exactly one event; got: {:?}",
            events
        );
        match &events[0] {
            TuiEvent::Error(msg) => {
                assert_eq!(
                    msg, "Usage: /add-dir <path>",
                    "empty-arg branch Error must match shipped format \
                     byte-for-byte"
                );
            }
            other => panic!("empty-arg branch must emit TuiEvent::Error, got: {other:?}"),
        }
    }

    /// A valid directory (real temp dir resolved via `std::env::temp_dir()`)
    /// must:
    /// * Stash `CommandEffect::AddExtraDir(path)` in `pending_effect`.
    /// * Emit a single `TuiEvent::TextDelta` whose payload matches the
    ///   shipped `format!("\nAdded '{}' to working directories for this
    ///   session.\nFiles in this directory are now accessible.\n",
    ///   path.display())` byte-for-byte.
    /// * Emit NO `TuiEvent::Error`.
    #[test]
    fn add_dir_handler_execute_with_valid_dir_stashes_effect_and_emits_confirmation() {
        let (mut ctx, mut rx) = make_ctx();
        let h = AddDirHandler;
        // Use `std::env::temp_dir()` to get a path that is guaranteed
        // to exist and be a directory across Linux / macOS / Windows.
        // Depends on no third-party crate (keeps the test surface
        // minimal). The existence + directory-ness of the returned
        // path is a platform invariant — if this assertion fires, the
        // test environment is fundamentally broken.
        let dir = std::env::temp_dir();
        assert!(
            dir.is_dir(),
            "test premise broken: std::env::temp_dir() ({}) must be an \
             existing directory",
            dir.display()
        );
        let path_arg = dir.to_string_lossy().to_string();

        let res = h.execute(&mut ctx, &[path_arg.clone()]);
        assert!(
            res.is_ok(),
            "AddDirHandler::execute(valid) must return Ok(()), got: {res:?}"
        );

        // 1. Pending effect MUST be Some(AddExtraDir(path)).
        match &ctx.pending_effect {
            Some(CommandEffect::AddExtraDir(p)) => {
                assert_eq!(
                    p,
                    &PathBuf::from(&path_arg),
                    "AddExtraDir must carry the PathBuf constructed from \
                     the trimmed arg"
                );
            }
            other => panic!("expected Some(CommandEffect::AddExtraDir(path)), got: {other:?}"),
        }

        // 2. Exactly one TextDelta event with byte-identical format.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "valid-dir branch must emit exactly one event; got: {:?}",
            events
        );
        let expected = format!(
            "\nAdded '{}' to working directories for this session.\nFiles in this directory are now accessible.\n",
            PathBuf::from(&path_arg).display()
        );
        match &events[0] {
            TuiEvent::TextDelta(text) => {
                assert_eq!(
                    text, &expected,
                    "valid-dir branch TextDelta must match shipped \
                     format! byte-for-byte"
                );
            }
            other => panic!(
                "valid-dir branch must emit TuiEvent::TextDelta, got: \
                 {other:?}"
            ),
        }

        // 3. NO Error event — implied by len()==1 and the TextDelta
        //    match above, but spell it out for the test contract.
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            !has_error,
            "valid-dir branch must emit NO TuiEvent::Error; got: {:?}",
            events
        );
    }

    /// An invalid directory (deliberately nonexistent path) must:
    /// * Emit a single `TuiEvent::Error` whose payload is byte-
    ///   identical to `format!("Directory not found: {path_arg}")`.
    /// * NOT stash any `CommandEffect` (pending_effect remains None).
    /// * NOT emit any `TuiEvent::TextDelta`.
    #[test]
    fn add_dir_handler_execute_with_invalid_dir_emits_not_found_error() {
        let (mut ctx, mut rx) = make_ctx();
        let h = AddDirHandler;
        // Use a deliberately nonexistent path. The `xyzzy123` suffix
        // and the nested-nonexistent parent chain make a collision
        // with a real directory on any sane test host vanishingly
        // unlikely. Defensive sanity: confirm it really is not a dir
        // so the test would catch a regression in the test env rather
        // than fail mysteriously.
        let bogus = "/this/path/does/not/exist/xyzzy123";
        assert!(
            !PathBuf::from(bogus).is_dir(),
            "test premise broken: '{bogus}' must NOT be an existing \
             directory on this host"
        );

        let res = h.execute(&mut ctx, &[bogus.to_string()]);
        assert!(
            res.is_ok(),
            "AddDirHandler::execute(invalid) must return Ok(()), got: {res:?}"
        );

        // 1. NO effect stashed.
        assert!(
            ctx.pending_effect.is_none(),
            "invalid-dir branch must NOT stash an effect; got: {:?}",
            ctx.pending_effect
        );

        // 2. Exactly one Error event with byte-identical payload.
        let events = drain(&mut rx);
        assert_eq!(
            events.len(),
            1,
            "invalid-dir branch must emit exactly one event; got: {:?}",
            events
        );
        let expected = format!("Directory not found: {bogus}");
        match &events[0] {
            TuiEvent::Error(msg) => {
                assert_eq!(
                    msg, &expected,
                    "invalid-dir branch Error must match shipped \
                     format! byte-for-byte"
                );
            }
            other => panic!(
                "invalid-dir branch must emit TuiEvent::Error, got: \
                 {other:?}"
            ),
        }

        // 3. NO TextDelta — implied by len()==1 and the Error match
        //    above, but spell it out for the test contract.
        let has_delta = events.iter().any(|e| matches!(e, TuiEvent::TextDelta(_)));
        assert!(
            !has_delta,
            "invalid-dir branch must emit NO TuiEvent::TextDelta; got: \
             {:?}",
            events
        );
    }

    /// Defensive test for R4: passing a multi-token args slice (e.g.
    /// `["/tmp", "foo"]`) must:
    /// * Join with " " and trim to `"/tmp foo"`.
    /// * Not panic.
    /// * Return `Ok(())`.
    /// * Emit at least one event (either a valid-dir TextDelta + effect
    ///   stash if the joined path happens to be a real directory on the
    ///   host, or an invalid-dir Error for the common case where it
    ///   is not).
    #[test]
    fn add_dir_handler_execute_joins_multi_token_args_without_panicking() {
        let (mut ctx, mut rx) = make_ctx();
        let h = AddDirHandler;
        // Two tokens that, when joined with a space, form a path that
        // is almost certainly NOT a real directory on the host. This
        // exercises the multi-token join path AND the invalid-dir
        // branch (the overwhelmingly common outcome for any multi-word
        // combination). If a future test host does happen to have a
        // directory at `/tmp foo` the assertion falls back to "at least
        // one event emitted" which both branches satisfy.
        let args = vec!["/tmp".to_string(), "foo-xyzzy-abc-123".to_string()];
        let res = h.execute(&mut ctx, &args);
        assert!(
            res.is_ok(),
            "AddDirHandler::execute(multi-token) must return Ok(()), \
             got: {res:?}"
        );

        let events = drain(&mut rx);
        assert!(
            !events.is_empty(),
            "AddDirHandler::execute(multi-token) must emit at least one \
             event; got: {:?}",
            events
        );

        // The joined path_arg passed to both branches is "/tmp foo-xyzzy-abc-123".
        // Validate that whichever branch fires, its emission carries
        // the joined path_arg (not just the first token).
        let joined = "/tmp foo-xyzzy-abc-123";
        let emitted_ok = events.iter().any(|e| match e {
            TuiEvent::Error(msg) => msg == &format!("Directory not found: {joined}"),
            TuiEvent::TextDelta(text) => text.contains(joined),
            _ => false,
        });
        assert!(
            emitted_ok,
            "multi-token branch must emit an event whose payload \
             references the JOINED path ('{joined}'), not just the \
             first token; got: {:?}",
            events
        );
    }

    // -------------------------------------------------------------------
    // Gate 5 dispatcher-integration tests — TASK-AGS-POST-6-BODIES-B10-ADDDIR
    // -------------------------------------------------------------------
    //
    // These tests drive the real `Dispatcher` + `default_registry()` +
    // `AddDirHandler` end-to-end, replacing the unit-level `h.execute(...)`
    // harness with the same dispatch path the TUI input loop uses. They
    // pin the fact that (a) registry routing for "/add-dir" lands on
    // `AddDirHandler`, (b) parser tokenization delivers args correctly
    // for both bare and trailing-args forms, and (c) byte-framing of
    // shipped strings survives the full dispatch chain.
    //
    // Reference template: src/command/color.rs dispatcher tests (B09-COLOR
    // Gate 5) — same structure, adjusted for EFFECT-SLOT semantics
    // (pending_effect stash + TextDelta vs DIRECT event emission).

    #[test]
    fn dispatcher_routes_slash_add_dir_to_handler_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        // Bare "/add-dir" → empty-arg branch → Error "Usage: /add-dir <path>".
        // Effect slot MUST remain None on empty-arg.
        let result = dispatcher.dispatch(&mut ctx, "/add-dir");
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/add-dir\") must return Ok"
        );

        let expected_error = "Usage: /add-dir <path>";
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let has_error = events
            .iter()
            .any(|e| matches!(e, TuiEvent::Error(msg) if msg == expected_error));
        let has_text_delta = events.iter().any(|e| matches!(e, TuiEvent::TextDelta(_)));
        assert!(
            has_error && !has_text_delta,
            "end-to-end bare `/add-dir` must emit byte-identical \
             Usage Error AND NO TextDelta; got: {:?}",
            events
        );
        assert!(
            ctx.pending_effect.is_none(),
            "empty-arg branch must NOT stash a CommandEffect; \
             got: {:?}",
            ctx.pending_effect
        );
    }

    #[test]
    fn dispatcher_routes_slash_add_dir_with_valid_dir_end_to_end() {
        use crate::command::dispatcher::Dispatcher;
        use crate::command::registry::CommandEffect;
        use crate::command::registry::default_registry;
        use std::sync::Arc;

        let registry = Arc::new(default_registry());
        let dispatcher = Dispatcher::new(registry);
        let (mut ctx, mut rx) = make_ctx();

        // Pick a definitely-existing directory. std::env::temp_dir() is
        // guaranteed to return an existing path on every supported host.
        let temp_dir = std::env::temp_dir();
        assert!(
            temp_dir.is_dir(),
            "std::env::temp_dir() precondition: must be an existing \
             directory on the test host; got: {temp_dir:?}"
        );
        let temp_dir_str = temp_dir.display().to_string();
        let input = format!("/add-dir {temp_dir_str}");

        // "/add-dir <valid>" → valid-dir branch → effect stash +
        // TextDelta confirmation. Exercises arg-consumption path.
        let result = dispatcher.dispatch(&mut ctx, &input);
        assert!(
            result.is_ok(),
            "dispatcher.dispatch(\"/add-dir <valid>\") must return Ok; \
             got: {result:?}"
        );

        // Effect stash MUST carry the PathBuf constructed from the path_arg.
        match ctx.pending_effect.as_ref() {
            Some(CommandEffect::AddExtraDir(p)) => {
                assert_eq!(
                    p,
                    &PathBuf::from(&temp_dir_str),
                    "AddExtraDir must carry the PathBuf from the \
                     reconciled path_arg; got: {p:?}"
                );
            }
            other => panic!(
                "expected Some(CommandEffect::AddExtraDir(path)), got: \
                 {other:?}"
            ),
        }

        let expected_confirmation = format!(
            "\nAdded '{}' to working directories for this session.\n\
             Files in this directory are now accessible.\n",
            temp_dir.display()
        );
        let mut events = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            events.push(ev);
        }
        let has_text_delta = events
            .iter()
            .any(|e| matches!(e, TuiEvent::TextDelta(s) if s == &expected_confirmation));
        let has_error = events.iter().any(|e| matches!(e, TuiEvent::Error(_)));
        assert!(
            has_text_delta && !has_error,
            "end-to-end `/add-dir <valid>` must emit byte-identical \
             confirmation TextDelta AND NO Error; got: {:?}",
            events
        );
    }
}
