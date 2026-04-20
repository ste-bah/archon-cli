//! TASK-AGS-POST-6-BODIES-B02-THINKING: /thinking slash-command handler
//! (Option C, DIRECT pattern body-migrate).
//!
//! Gate 1 skeleton — the real `impl CommandHandler for ThinkingHandler`
//! lands at Gate 2. This file currently contains only the inline test
//! skeleton with `#[ignore]` + `todo!()` bodies so Cargo compiles the
//! module but no test assertions execute. Gate 2 lands:
//! - `pub(crate) struct ThinkingHandler;` + `impl CommandHandler`
//! - Replaces each `#[ignore] + todo!()` test with real assertions
//! - Moves the existing `declare_handler!` stub at registry.rs:587 to
//!   a breadcrumb comment (same treatment as B01-FAST)
//!
//! See docs/stage-7.5/tickets/TASK-AGS-POST-6-BODIES-B02-THINKING.md
//! for the full 6-gate plan.

#[cfg(test)]
mod tests {
    // Gate 1 skeleton. Each test documents its Gate 2 assertion via
    // the `#[ignore = "..."]` message. Gate 2 replaces the `todo!()`
    // body with real assertions using `make_thinking_ctx` (helper
    // added to test_support.rs at Gate 2).

    #[test]
    #[ignore = "Gate 2: args=[\"on\"] must transition show_thinking false->true AND emit TuiEvent::ThinkingToggle(true) THEN TuiEvent::TextDelta(\"\\nThinking display enabled.\\n\") (order matters)"]
    fn thinking_handler_on_enables_and_emits_events() {
        todo!("Gate 2: assert ThinkingHandler.execute(ctx, &[\"on\"]) flips atomic false->true, emits ThinkingToggle(true) then TextDelta containing \"Thinking display enabled.\"")
    }

    #[test]
    #[ignore = "Gate 2: args=[\"off\"] must transition show_thinking true->false AND emit TuiEvent::ThinkingToggle(false) THEN TuiEvent::TextDelta(\"\\nThinking display disabled.\\n\") (order matters)"]
    fn thinking_handler_off_disables_and_emits_events() {
        todo!("Gate 2: assert ThinkingHandler.execute(ctx, &[\"off\"]) flips atomic true->false, emits ThinkingToggle(false) then TextDelta containing \"Thinking display disabled.\"")
    }

    #[test]
    #[ignore = "Gate 2: empty args must default to enable (preserves legacy \"/thinking\" alone = enable, per slash.rs:75)"]
    fn thinking_handler_empty_args_defaults_to_enable() {
        todo!("Gate 2: assert ThinkingHandler.execute(ctx, &[]) with initial false transitions to true AND emits enabled events — mirrors legacy `\"/thinking on\" | \"/thinking\"` arm at slash.rs:75")
    }

    #[test]
    #[ignore = "Gate 2: unknown args (e.g., \"foo\") must be silent no-op — no state change, no events (preserves legacy fall-through)"]
    fn thinking_handler_unknown_arg_is_silent_noop() {
        todo!("Gate 2: assert ThinkingHandler.execute(ctx, &[\"foo\"]) leaves show_thinking UNCHANGED AND emits NO TuiEvents (rx.try_recv() returns Empty)")
    }
}
