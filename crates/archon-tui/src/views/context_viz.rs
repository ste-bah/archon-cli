//! Path A stub-scaffold for TASK-AGS-618 (context visualization overlay).
//!
//! This is the initial scaffolding for the context visualization overlay.
//! The full migration — wiring an `app.rs` `ContextVizState` field, binding a
//! key in `crate::input` to open the overlay, sourcing real `Message` values
//! from the chat view's message list (rather than the local `TurnUsage` stub
//! type defined here), running the real tokenizer to produce per-turn input
//! and output token counts (rather than relying on caller-supplied counts),
//! integrating the crate's palette theming (accent / fg / muted) into the
//! drawn block, allowing the overlay to render on top of other views, and
//! wiring the `/cost` slash command in Phase 8 — is intentionally deferred
//! to a later phase. See `CONTEXT_VIZ_PLACEHOLDER`.
//!
//! For now this module exposes the minimal API surface (`Role`, `TurnUsage`,
//! `ContextVizState`, `open`, `draw`, `on_key`, `on_agent_event`, `update`,
//! `bar_width`) so subsequent work and tests can depend on it. Per-turn
//! usage rows are stored as a plain in-memory `Vec<TurnUsage>` populated by
//! the caller; the stub does NOT reach into the real chat view message list
//! and does NOT run any real tokenizer.
//!
//! Per the per-view isolation rule, this module MUST NOT import from any
//! other `crate::views::*` module. It also MUST NOT import any new
//! crate-level dependencies (no real `Message` type, no tokenizer crate,
//! no palette theming module beyond what ratatui already provides) — those
//! arrive with the full migration alongside the `app.rs` `ContextVizState`
//! field, the `input.rs` key binding, and the `/cost` slash command.

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};

/// Placeholder for the full context visualization overlay content.
///
/// The real `Message` type integration, real tokenizer integration,
/// `app.rs` `ContextVizState` field, `input.rs` key binding, palette
/// theming (accent / fg / muted), overlay-over-other-views interaction,
/// and `/cost` slash command (Phase 8) are deferred to a later task. This
/// constant exists so future modules and tests have a single symbol to
/// point at when wiring context-visualization content.
pub const CONTEXT_VIZ_PLACEHOLDER: &str =
    "Context visualization (placeholder — app.rs ContextVizState field + input.rs keybinding + real Message type integration + real tokenizer + palette theming + overlay-over-other-views + /cost slash command deferred)";

/// Speaker role for a single conversational turn.
///
/// Deliberately a small local enum rather than the canonical chat
/// `Message` role type — Path A defers the real `Message` type
/// integration. The full migration will replace this enum with the
/// canonical role and adjust call sites accordingly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Turn authored by the human user.
    User,
    /// Turn authored by the assistant model.
    Assistant,
    /// System / tool / framework turn (instructions, tool output, etc.).
    System,
}

impl Default for Role {
    fn default() -> Self {
        Self::User
    }
}

/// Per-turn token usage row shown by the context visualization overlay.
///
/// Carries the input/output token counts for a single conversational
/// turn alongside that turn's `Role`. The caller is expected to populate
/// `tokens_in` / `tokens_out` — Path A defers the real tokenizer
/// integration, so this stub does NOT compute either count itself (no
/// char-counting is performed). The full migration will replace this
/// type with a view onto the real `Message` type and source token counts
/// from the real tokenizer.
#[derive(Debug, Clone, Default)]
pub struct TurnUsage {
    /// Input (prompt) token count for this turn, as supplied by the
    /// caller. The stub does not validate or recompute it.
    pub tokens_in: usize,
    /// Output (completion) token count for this turn, as supplied by
    /// the caller. The stub does not validate or recompute it.
    pub tokens_out: usize,
    /// Speaker role for this turn. Defaults to `Role::User`.
    pub role: Role,
}

/// State for the context visualization overlay.
///
/// Tracks the available per-turn usage rows, the configured context
/// window size (in tokens — read by the future `bar_width` rendering
/// path to scale per-turn bars), and the cursor position over those
/// rows. The stub never populates `turns` on its own — it exists so
/// call sites stay stable across the full migration.
#[derive(Debug, Default, Clone)]
pub struct ContextVizState {
    /// All per-turn usage rows currently shown by the overlay. Empty
    /// by default; the caller is expected to populate this via
    /// [`update`] before invoking [`open`].
    pub turns: Vec<TurnUsage>,
    /// Configured context window size in tokens. Used by [`bar_width`]
    /// to scale per-turn bars; the stub does not source this from any
    /// real model configuration.
    pub window_size: usize,
    /// Index of the currently highlighted row. Always clamped to
    /// `0..turns.len()` (or `0` when `turns` is empty).
    pub cursor: usize,
}

/// Open (or re-open) the context visualization overlay.
///
/// Stub behaviour: resets `cursor` to `0` so the highlight always
/// lands on the top row when the overlay becomes visible. Does NOT
/// touch `turns` (the caller owns that list via [`update`]) or
/// `window_size` (the caller / future migration owns that).
///
/// The full migration will additionally refresh `turns` from the real
/// chat view message list and re-run the real tokenizer to produce
/// fresh per-turn token counts.
pub fn open(state: &mut ContextVizState) {
    state.cursor = 0;
}

/// Draw the context visualization overlay into `area`.
///
/// Renders an empty bordered block titled "Context" — actual per-turn
/// bar rendering, role colouring via palette theming (accent / fg /
/// muted), header summary (used / window / percent), and footer hints
/// are deferred to the full migration. The stub deliberately survives
/// an empty `turns` list without panicking so the overlay can be drawn
/// before any [`update`] has populated it.
pub fn draw(frame: &mut Frame, area: Rect, _state: &ContextVizState) {
    let block = Block::default().borders(Borders::ALL).title("Context");
    frame.render_widget(block, area);
}

/// Handle a key event for the context visualization overlay.
///
/// Returns `false` because this scaffold does not yet consume input
/// via the parent app event loop in any meaningful way — the
/// overlay-over-other-views interaction is deferred to the full
/// migration. Cursor movement is clamped to `0..turns.len()`:
///
/// * `Down` / `j` — advance cursor by one row (clamped at the end)
/// * `Up`   / `k` — retreat cursor by one row (saturating at zero)
/// * `Esc`        — no-op; overlay close is handled by the parent app
///                  when wiring lands
///
/// All other keys are ignored. The full migration will replace this
/// with the real keymap dispatched through `input.rs`.
pub fn on_key(state: &mut ContextVizState, key_code: KeyCode) -> bool {
    match key_code {
        KeyCode::Down | KeyCode::Char('j') => {
            let max_index = state.turns.len().saturating_sub(1);
            if state.cursor < max_index {
                state.cursor += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.cursor = state.cursor.saturating_sub(1);
        }
        KeyCode::Esc => {
            // No-op stub: the overlay close is handled by the parent
            // app when the deferred wiring lands.
        }
        _ => {}
    }
    false
}

/// React to an agent event.
///
/// No-op stub. The full implementation will react to per-turn
/// usage updates emitted by the chat view (and, eventually, by the
/// `/cost` slash command in Phase 8) by replacing `turns` in place.
/// Those event variants do NOT yet exist on `crate::app::TuiEvent` —
/// Path A explicitly defers adding them, along with the `app.rs`
/// `ContextVizState` field and the `input.rs` key binding.
pub fn on_agent_event(_state: &mut ContextVizState) {}

/// Replace the overlay's per-turn usage rows and configured window size.
///
/// STUB: this function takes a `&[TurnUsage]` slice rather than a real
/// `Message` slice — the real `Message` type integration is deferred,
/// so the local stub type is what the call signature accepts today.
/// It also takes `window_size` directly from the caller rather than
/// inferring it from any real model configuration. The stub does NO
/// char-counting and does NOT run any tokenizer; the caller has
/// already computed `tokens_in` / `tokens_out` for each turn. The
/// `CONTEXT_VIZ_PLACEHOLDER` constant documents the deferred tokenizer
/// integration.
///
/// The full migration will:
///   * Replace the `&[TurnUsage]` slice with a view onto the real
///     `Message` type sourced from the chat view's message list.
///   * Run the real tokenizer to produce per-turn `tokens_in` /
///     `tokens_out` rather than trusting the caller.
///   * Source `window_size` from the real model configuration rather
///     than letting the caller pass it in directly.
pub fn update(state: &mut ContextVizState, turns: &[TurnUsage], window_size: usize) {
    state.turns = turns.to_vec();
    state.window_size = window_size;
}

/// Compute the rendered width (in cells) of a per-turn token bar.
///
/// Helper used by the future bar-rendering path. Scales `tokens` into
/// the range `0..=max_width` by dividing by `window_size`. Uses
/// saturating arithmetic throughout so it never panics:
///
///   * If `window_size == 0`, returns `0` (no panic on divide-by-zero).
///   * If `tokens >= window_size`, returns `max_width` (clamped at the
///     top end).
///   * Otherwise returns `(tokens / window_size) * max_width`, computed
///     in `u128` to avoid overflow on large token counts before being
///     cast back into the `0..=max_width` range.
pub fn bar_width(tokens: usize, window_size: usize, max_width: u16) -> u16 {
    if window_size == 0 {
        return 0;
    }
    if tokens >= window_size {
        return max_width;
    }
    // Scale into the 0..=max_width range using u128 to avoid overflow.
    let scaled = (tokens as u128).saturating_mul(max_width as u128) / (window_size as u128);
    if scaled > max_width as u128 {
        max_width
    } else {
        scaled as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;

    fn turn(tokens_in: usize, tokens_out: usize, role: Role) -> TurnUsage {
        TurnUsage {
            tokens_in,
            tokens_out,
            role,
        }
    }

    fn populated_state() -> ContextVizState {
        ContextVizState {
            turns: vec![
                turn(100, 200, Role::User),
                turn(150, 250, Role::Assistant),
                turn(50, 0, Role::System),
            ],
            window_size: 8000,
            cursor: 0,
        }
    }

    #[test]
    fn context_viz_state_default_empty() {
        let state = ContextVizState::default();
        assert!(state.turns.is_empty(), "default turns should be empty");
        assert_eq!(state.window_size, 0, "default window_size should be 0");
        assert_eq!(state.cursor, 0, "default cursor should be 0");
    }

    #[test]
    fn on_key_down_advances_cursor() {
        let mut state = populated_state();
        let consumed = on_key(&mut state, KeyCode::Down);
        assert!(!consumed, "stub on_key should not consume input");
        assert_eq!(state.cursor, 1, "Down should advance cursor to 1");

        // 'j' should behave identically to Down.
        let mut state_j = populated_state();
        on_key(&mut state_j, KeyCode::Char('j'));
        assert_eq!(state_j.cursor, 1, "j should advance cursor like Down");
    }

    #[test]
    fn on_key_down_clamps_at_end() {
        let mut state = populated_state();
        state.cursor = state.turns.len() - 1;
        on_key(&mut state, KeyCode::Down);
        assert_eq!(
            state.cursor,
            state.turns.len() - 1,
            "Down at last row should keep cursor clamped at turns.len()-1"
        );
    }

    #[test]
    fn on_key_up_saturates_at_zero() {
        let mut state = populated_state();
        on_key(&mut state, KeyCode::Up);
        assert_eq!(state.cursor, 0, "Up at cursor 0 should saturate at 0");

        // 'k' should behave identically to Up.
        let mut state_k = populated_state();
        on_key(&mut state_k, KeyCode::Char('k'));
        assert_eq!(state_k.cursor, 0, "k at cursor 0 should saturate at 0");
    }

    #[test]
    fn on_key_esc_is_noop() {
        let mut state = populated_state();
        state.cursor = 1;
        let consumed = on_key(&mut state, KeyCode::Esc);
        assert!(!consumed, "Esc should return false (stub does not consume)");
        assert_eq!(
            state.cursor, 1,
            "Esc should not mutate cursor beyond documented behaviour"
        );
        assert_eq!(
            state.turns.len(),
            3,
            "Esc should not mutate turns beyond documented behaviour"
        );
        assert_eq!(
            state.window_size, 8000,
            "Esc should not mutate window_size beyond documented behaviour"
        );
    }

    #[test]
    fn open_resets_cursor_to_zero() {
        let mut state = populated_state();
        state.cursor = 2;
        open(&mut state);
        assert_eq!(state.cursor, 0, "open should reset cursor to 0");
        assert_eq!(
            state.turns.len(),
            3,
            "open should not touch turns list"
        );
        assert_eq!(
            state.window_size, 8000,
            "open should not touch window_size"
        );
    }

    #[test]
    fn update_replaces_turns_and_window_size() {
        let mut state = ContextVizState::default();
        let new_turns = vec![
            turn(10, 20, Role::User),
            turn(30, 40, Role::Assistant),
        ];
        update(&mut state, &new_turns, 4096);
        assert_eq!(state.turns.len(), 2, "update should replace turns");
        assert_eq!(state.turns[0].tokens_in, 10);
        assert_eq!(state.turns[0].tokens_out, 20);
        assert_eq!(state.turns[0].role, Role::User);
        assert_eq!(state.turns[1].tokens_in, 30);
        assert_eq!(state.turns[1].tokens_out, 40);
        assert_eq!(state.turns[1].role, Role::Assistant);
        assert_eq!(
            state.window_size, 4096,
            "update should set window_size to the supplied value"
        );
    }

    #[test]
    fn update_handles_empty_turns() {
        let mut state = populated_state();
        update(&mut state, &[], 8000);
        assert!(
            state.turns.is_empty(),
            "update with empty slice should clear turns"
        );
        assert_eq!(
            state.window_size, 8000,
            "update with empty slice should still set window_size"
        );
    }

    #[test]
    fn bar_width_clamps_to_max() {
        // tokens >= window_size should saturate at max_width.
        assert_eq!(bar_width(8000, 8000, 100), 100);
        assert_eq!(bar_width(16000, 8000, 100), 100);
        assert_eq!(bar_width(usize::MAX, 8000, 50), 50);
    }

    #[test]
    fn bar_width_zero_window_returns_zero() {
        // window_size == 0 must not panic and must return 0.
        assert_eq!(bar_width(0, 0, 100), 0);
        assert_eq!(bar_width(1234, 0, 100), 0);
        assert_eq!(bar_width(usize::MAX, 0, 100), 0);
    }

    #[test]
    fn bar_width_fractional_scales_correctly() {
        // tokens=2000, window=8000, max_width=100 -> 25
        assert_eq!(bar_width(2000, 8000, 100), 25);
        // tokens=4000, window=8000, max_width=100 -> 50
        assert_eq!(bar_width(4000, 8000, 100), 50);
        // tokens=1, window=8000, max_width=100 -> 0 (integer division)
        assert_eq!(bar_width(1, 8000, 100), 0);
    }

    #[test]
    fn draw_does_not_panic_empty() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = ContextVizState::default();
        terminal
            .draw(|f| draw(f, f.area(), &state))
            .expect("draw should not panic on empty state");
    }

    #[test]
    fn draw_does_not_panic_populated() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = populated_state();
        terminal
            .draw(|f| draw(f, f.area(), &state))
            .expect("draw should not panic on populated state");
    }
}
