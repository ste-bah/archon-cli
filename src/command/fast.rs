//! TASK-AGS-POST-6-BODIES-B01-FAST: /fast slash-command handler (body-migrate target).
//!
//! This file is created in Gate 1 (tests-written-first) with the test
//! module ONLY. The production `impl CommandHandler for FastHandler` is
//! intentionally deferred to Gate 2 (implementation-complete), which
//! will:
//!   1. Lift the body verbatim from `src/command/slash.rs:60-70`
//!      (legacy `/fast` match arm).
//!   2. Remove `declare_handler!(FastHandler, ...)` from
//!      `src/command/registry.rs:546` and replace it with a
//!      `pub(crate) use crate::command::fast::FastHandler;` import.
//!   3. Add a `fast_mode_shared: Option<Arc<AtomicBool>>` field (or
//!      equivalent) to [`crate::command::registry::CommandContext`] and
//!      a `make_fast_ctx()` helper to
//!      `src/command/test_support.rs`.
//!   4. Remove the `#[ignore]` markers below and replace the `todo!()`
//!      bodies with real assertions that call
//!      `FastHandler.execute(&mut ctx, &[])`.
//!
//! Pattern: DIRECT (sync atomic write + TuiEvent send). No snapshot,
//! no effect-slot required (see ticket "Pattern Verification" section).
//!
//! Format strings (lifted from slash.rs:60-70, preserved byte-for-byte
//! in Gate 2):
//!   ENABLED : `"Fast mode ENABLED. Responses will be faster but lower quality."`
//!   DISABLED: `"Fast mode DISABLED. Back to normal quality."`
//!   Emission: `TuiEvent::TextDelta(format!("\n{msg}\n"))`

// NOTE: No production items live in this file at Gate 1. The
// `FastHandler` struct remains at `registry.rs:546` via the
// `declare_handler!` macro until Gate 2 performs the lift. This keeps
// the binary crate compiling: removing the macro line without also
// landing a real `impl CommandHandler` would break
// `b.insert_primary("fast", Arc::new(FastHandler))` at registry.rs:661.

#[cfg(test)]
mod tests {
    // Gate 1 skeleton tests (Path A-variant per ticket — #[ignore] markers
    // documented in Gate 1 body). Gate 2 lands impl, removes #[ignore],
    // replaces todo!() bodies with real assertions that reference
    // FastHandler::execute and the Gate-2-added fast_mode_shared field on
    // CommandContext.
    //
    // The tests compile today because todo!() satisfies any return type
    // and #[ignore] prevents them from running under `cargo test` (Gate 4
    // target count stays at the 174 + N with N = 3 declared, and the
    // tests appear in the `ok` list as `IGNORED`).
    //
    // Once Gate 2 lands, the standard inline-test fixture import appears:
    //     use super::*;
    //     use crate::command::test_support::*;
    // Keeping them commented here so the imports don't trigger
    // `unused_import` warnings at Gate 1.

    #[test]
    #[ignore = "Gate 2 (TASK-AGS-POST-6-BODIES-B01-FAST) lands FastHandler impl; \
                this test will call FastHandler.execute(&mut ctx, &[]) with \
                fast_mode_shared initialised to false, then assert \
                (a) the shared AtomicBool transitions to true, and \
                (b) a TuiEvent::TextDelta is emitted whose payload contains \
                the exact substring \"Fast mode ENABLED\" (from slash.rs:64)."]
    fn fast_handler_toggle_enables_when_initial_disabled() {
        todo!(
            "Gate 2: build CommandContext via make_fast_ctx with \
             fast_mode_shared=false; invoke FastHandler.execute(&mut ctx, &[]); \
             assert ctx.fast_mode_shared.load(Ordering::Relaxed) == true AND \
             a TuiEvent::TextDelta was received whose payload contains \
             \"Fast mode ENABLED\" (full expected literal: \
             \"Fast mode ENABLED. Responses will be faster but lower quality.\" \
             wrapped in `\\n{{msg}}\\n`, per slash.rs:60-70)."
        )
    }

    #[test]
    #[ignore = "Gate 2 (TASK-AGS-POST-6-BODIES-B01-FAST) lands FastHandler impl; \
                this test will call FastHandler.execute(&mut ctx, &[]) with \
                fast_mode_shared initialised to true, then assert \
                (a) the shared AtomicBool transitions to false, and \
                (b) a TuiEvent::TextDelta is emitted whose payload contains \
                the exact substring \"Fast mode DISABLED\" (from slash.rs:66)."]
    fn fast_handler_toggle_disables_when_initial_enabled() {
        todo!(
            "Gate 2: build CommandContext via make_fast_ctx with \
             fast_mode_shared=true; invoke FastHandler.execute(&mut ctx, &[]); \
             assert ctx.fast_mode_shared.load(Ordering::Relaxed) == false AND \
             a TuiEvent::TextDelta was received whose payload contains \
             \"Fast mode DISABLED\" (full expected literal: \
             \"Fast mode DISABLED. Back to normal quality.\" \
             wrapped in `\\n{{msg}}\\n`, per slash.rs:60-70)."
        )
    }

    #[test]
    #[ignore = "Gate 2 (TASK-AGS-POST-6-BODIES-B01-FAST) lands FastHandler impl; \
                this test will invoke FastHandler.execute twice sequentially \
                starting from an arbitrary initial state (say false), then \
                assert the shared AtomicBool returns to the original value \
                (toggle idempotence over two calls: A -> !A -> A)."]
    fn fast_handler_second_invocation_returns_opposite_state() {
        todo!(
            "Gate 2: build CommandContext via make_fast_ctx with \
             fast_mode_shared=false; invoke FastHandler.execute(&mut ctx, &[]) \
             twice sequentially; assert after first call \
             ctx.fast_mode_shared.load(Ordering::Relaxed) == true and after \
             second call it returns to false. Also drain the TuiEvent \
             receiver and assert both events are TuiEvent::TextDelta — the \
             first containing \"Fast mode ENABLED\", the second containing \
             \"Fast mode DISABLED\"."
        )
    }
}
