//! TASK-AGS-POST-6-BODIES-B05-VIM: /vim slash-command handler
//! (Option C, DIRECT pattern body-migrate).
//!
//! Reference: src/command/slash.rs:407 (shipped `/vim` match body —
//!   arm deletion is Gate 3 scope)
//! Based on: src/command/registry.rs:728 (`declare_handler!(VimHandler,
//!   "Toggle vim-style modal input")` no-op stub being replaced)
//! Source: src/command/fast.rs (B01-FAST DIRECT precedent — emit-only
//!   sync handler returning `Ok(())` via `ctx.tui_tx.try_send`).
//!
//! Gate 1 test skeleton — 5 `#[ignore]` + `todo!()` stubs. Real
//! `impl CommandHandler for VimHandler` (moved out of registry.rs's
//! `declare_handler!` macro) and de-ignored tests land at Gate 2.
//!
//! # Why DIRECT (no snapshot, no effect slot)
//!
//! Shipped body at slash.rs:407 performs two sequential `.send().await`
//! calls on `tui_tx`: `VimToggle` (which the TUI interprets as the
//! canonical toggle signal) and a `TextDelta` with the persist-hint
//! message. Both emissions become sync `try_send` calls on
//! `ctx.tui_tx` — the standard DIRECT-pattern replacement (matches
//! B01-FAST, B02-THINKING, B03-BUG precedent). Dropped messages under
//! 16-cap channel backpressure are preferable to stalling the
//! dispatcher; `/vim` output is best-effort informational UI.
//!
//! No state mutation happens in the handler (the TUI owns the vim-mode
//! bool, reacting to `VimToggle`); no new `CommandContext` field is
//! required. Simpler than B01-FAST (no shared atomic) and B04-DIFF (no
//! effect slot).
//!
//! # Byte-for-byte output preservation
//!
//! - `TuiEvent::VimToggle` — enum variant, no string payload.
//! - `TuiEvent::TextDelta` payload preserved verbatim:
//!   `"\nVim mode toggled. To persist: set vim_mode = true under [tui] in config.toml\n"`
//!
//! # Trailing-args policy
//!
//! Shipped arm matches exactly `"/vim"`; `/vim on` would have fallen
//! through to the default "unknown command" handler. Post-migration,
//! all `/vim*` inputs route to `VimHandler` via the registry. Chosen
//! strategy: **ignore trailing args and always emit both events** —
//! mirrors the B03-BUG / B04-DIFF trailing-args promotion. Simpler
//! code, better UX.
//!
//! # Aliases
//!
//! Shipped pre-B05-VIM: none. Spec lists none. No aliases added.

#[cfg(test)]
mod tests {
    // Gate 1 skeleton — 5 `#[ignore]` + `todo!()` stubs. Real
    // assertions land at Gate 2 once the `declare_handler!(VimHandler,
    // ...)` macro stub at `registry.rs:728` is replaced by a real
    // `impl CommandHandler for VimHandler` block moved into THIS file,
    // alongside the `insert_primary("vim", ...)` wiring at
    // `registry.rs:812` (which flips from the macro-exported stub to
    // `crate::command::vim::VimHandler`).
    //
    // N=5 tests — covers the DIRECT emit matrix plus byte-identity
    // pins:
    //   1. args=[]          → TuiEvent::VimToggle emitted
    //   2. args=[]          → TuiEvent::TextDelta with persist-hint emitted
    //   3. args=["on"]      → same two events emitted (trailing args ignored)
    //   4. VimHandler.description() byte-identical to shipped literal
    //      "Toggle vim-style modal input"
    //   5. execute(&mut ctx, &[]) returns Ok(())

    #[test]
    #[ignore = "Gate 2: args=[] must emit TuiEvent::VimToggle as the first \
                event on ctx.tui_tx (canonical toggle signal consumed by \
                the TUI)"]
    fn vim_handler_emits_vim_toggle_event() {
        todo!(
            "Gate 2: VimHandler.execute(&mut ctx, &[]) -> Ok(()); \
             drain_tui_events(&mut rx) must include TuiEvent::VimToggle \
             (first event emitted)"
        )
    }

    #[test]
    #[ignore = "Gate 2: args=[] must emit TuiEvent::TextDelta with payload \
                byte-identical to the shipped slash.rs:411 literal \
                '\\nVim mode toggled. To persist: set vim_mode = true \
                under [tui] in config.toml\\n'"]
    fn vim_handler_emits_text_delta_with_persist_message() {
        todo!(
            "Gate 2: VimHandler.execute(&mut ctx, &[]) -> Ok(()); \
             drain_tui_events must include exactly one TuiEvent::TextDelta \
             whose payload equals \"\\nVim mode toggled. To persist: set \
             vim_mode = true under [tui] in config.toml\\n\""
        )
    }

    #[test]
    #[ignore = "Gate 2: args=[\"on\"] must emit the SAME two events as \
                args=[] case (VimToggle + TextDelta). Trailing-args \
                ignored; B03-BUG / B04-DIFF promotion policy."]
    fn vim_handler_ignores_trailing_args() {
        todo!(
            "Gate 2: VimHandler.execute(&mut ctx, &[String::from(\"on\")]) \
             -> Ok(()); drain_tui_events must contain both VimToggle AND \
             the byte-identical TextDelta payload (same emission sequence \
             as args=[] case)"
        )
    }

    #[test]
    #[ignore = "Gate 2: VimHandler.description() must return the \
                byte-identical shipped string \"Toggle vim-style modal \
                input\"; replaces the declare_handler! macro arg at \
                registry.rs:728"]
    fn vim_handler_description_byte_identical_to_shipped() {
        todo!(
            "Gate 2: VimHandler.description() -> \
             \"Toggle vim-style modal input\" (byte-identical to shipped \
             declare_handler! arg at registry.rs:728)"
        )
    }

    #[test]
    #[ignore = "Gate 2: execute(&mut ctx, &[]) must return Ok(()) \
                unconditionally — /vim has no error path (no shared \
                state required, no validation, no args parsing)"]
    fn vim_handler_execute_returns_ok() {
        todo!(
            "Gate 2: VimHandler.execute(&mut ctx, &[]) returns Ok(()) \
             (no Err branch; unconditional emit-only handler)"
        )
    }
}
