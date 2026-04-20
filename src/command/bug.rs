//! TASK-AGS-POST-6-BODIES-B03-BUG: /bug slash-command handler
//! (Option C, DIRECT pattern body-migrate — trivial variant).
//!
//! Gate 1 skeleton. The real `CommandHandler` impl lands at Gate 2,
//! replacing the `declare_handler!(BugHandler, ...)` stub at
//! `src/command/registry.rs:658` and the legacy match arm at
//! `src/command/slash.rs:349-356`.
//!
//! # Why DIRECT (trivial variant)
//!
//! The shipped `/bug` body at slash.rs:349-356 is the simplest possible
//! slash-command body in the codebase:
//!
//! ```ignore
//! "/bug" => {
//!     let _ = tui_tx
//!         .send(TuiEvent::TextDelta(
//!             "\nReport bugs at https://github.com/anthropics/archon/issues\n".into(),
//!         ))
//!         .await;
//!     true
//! }
//! ```
//!
//! - NO args (exact-match `"/bug"` arm — no subcommand parse)
//! - NO shared state (no `Arc<AtomicBool>`, no `Arc<Mutex<T>>`)
//! - NO `CommandContext` field added (no new cross-cutting plumbing)
//! - NO snapshot (no read-side state to pre-compute)
//! - NO `CommandEffect` variant (no write-side state to defer)
//! - NO preceding `ThinkingToggle`-style event — SINGLE TextDelta
//! - NO aliases — shipped pre-B03-BUG stub carries none; spec lists none
//!
//! Simpler than B01-FAST (which had an atomic toggle) and B02-THINKING
//! (which had subcommand parsing + two-event emission). The only
//! structural change from shipped is the emission primitive:
//! `tui_tx.send(..).await` → `ctx.tui_tx.try_send(..)` per B01/B02
//! precedent. The emitted TextDelta string is byte-identical to the
//! shipped literal at slash.rs:351-353 — Sherlock Gate 3 will MD5-verify.
//!
//! # Byte-for-byte output preservation
//!
//! Migrated string (Gate 2 impl will reproduce this exactly):
//! ```ignore
//! "\nReport bugs at https://github.com/anthropics/archon/issues\n"
//! ```
//!
//! The leading and trailing `\n` wrap MUST be preserved.
//!
//! # Trailing-args policy (decision rationale)
//!
//! The shipped arm at slash.rs:349 matches exactly `"/bug"` and would
//! fall through for `"/bug foo"` to the default "unknown command"
//! handler. Post-migration, ALL `/bug*` inputs route to `BugHandler`
//! via the registry. The chosen preservation strategy is: **ignore
//! trailing args and always emit the URL**. Simpler code, better UX
//! (the user gets help even if they add arguments), and the legacy
//! fall-through was a dispatch quirk, not a documented contract. This
//! is a mild semantic promotion from the shipped behavior, documented
//! in the ticket at TASK-AGS-POST-6-BODIES-B03-BUG.md lines 130-140.
//!
//! The Gate 2 test `bug_handler_ignores_trailing_args` pins this
//! contract — passing `args=["foo"]` must produce the same single
//! TextDelta as `args=[]`.

#[cfg(test)]
mod tests {
    // Gate 1 skeleton — #[ignore] + todo!() stubs. Real impl + real
    // assertions land at Gate 2 once `BugHandler` exists and the
    // `make_bug_ctx` helper is added to `test_support.rs`.
    //
    // N=2 tests — minimal matrix for a trivial-variant DIRECT handler
    // with no args, no state, no subcommand branches. Path A-variant
    // inline tests (colocated with the handler impl, matching the
    // B01-FAST and B02-THINKING precedent).

    #[test]
    #[ignore = "Gate 2: args=[] must emit exactly one TuiEvent::TextDelta \
                containing 'Report bugs at https://github.com/anthropics/archon/issues' \
                AND the leading+trailing `\\n` wrap; no other events \
                (no ThinkingToggle, no second emission)"]
    fn bug_handler_execute_emits_bug_url_textdelta() {
        todo!(
            "Gate 2: BugHandler.execute(&mut ctx, &[]) -> Ok(()); \
             drain_tui_events(&mut rx) -> assert exactly one \
             TuiEvent::TextDelta whose payload == \
             \"\\nReport bugs at https://github.com/anthropics/archon/issues\\n\" \
             (byte-identical to shipped slash.rs:351-353)"
        )
    }

    #[test]
    #[ignore = "Gate 2: args=[\"foo\"] must produce the SAME single \
                TextDelta as args=[] (trailing args ignored, always \
                emit URL — preserves B02-THINKING-style silent-noop \
                semantics for 'additional tokens ignored' in a trivial \
                handler that takes no args). Decision rationale: \
                ticket B03 lines 130-140 — mild semantic promotion \
                from shipped exact-match-only arm to always-emit"]
    fn bug_handler_ignores_trailing_args() {
        todo!(
            "Gate 2: BugHandler.execute(&mut ctx, &[String::from(\"foo\")]) \
             -> Ok(()); drain_tui_events(&mut rx) -> assert exactly \
             one TuiEvent::TextDelta with the SAME byte content as \
             the args=[] case (same URL string, same `\\n` wrap)"
        )
    }
}
