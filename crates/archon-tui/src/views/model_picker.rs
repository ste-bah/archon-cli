//! Model picker overlay view (TASK-AGS-619 stub scaffold — Path A).
//!
//! This is the initial scaffolding for the model picker overlay. The full
//! migration — wiring a `TuiEvent::SwitchModel` variant through the app
//! event loop, adding a `ModelPickerState` field on `crate::app::App`,
//! binding a key (e.g. `Ctrl-M`) in `crate::input`, pulling real provider
//! and model entries from the provider registry shipped by
//! `TECH-AGS-PROVIDERS` Phase 7, and implementing the `ERR-TUI-01`
//! "provider unreachable -> fall back to previous selection" path — is
//! intentionally deferred to a later phase. See `MODEL_PICKER_PLACEHOLDER`.
//!
//! For now this module exposes the minimal API surface
//! (`ProviderId`, `ModelId`, `ProviderEntry`, `ModelPickerState`, `open`,
//! `draw`, `on_key`, `on_agent_event`) so subsequent work and tests can
//! depend on it. Provider entries are stored as plain in-memory
//! `Vec<ProviderEntry>` populated by the caller; the stub does NOT reach
//! out to any real provider registry, and `ProviderId` / `ModelId` are
//! deliberately local `String` aliases rather than the real types that
//! `TECH-AGS-PROVIDERS` will introduce.
//!
//! Per the per-view isolation rule, this module MUST NOT import from any
//! other `crate::views::*` module. It also MUST NOT import any new
//! crate-level dependencies (no `archon_session`, no provider-registry
//! crate) — those arrive with the full migration alongside the
//! `TuiEvent::SwitchModel` variant and the corresponding `app.rs` /
//! `input.rs` wiring.

use ratatui::Frame;
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};

/// Placeholder for the full model picker view content.
///
/// The real provider-registry integration, `TuiEvent::SwitchModel`
/// plumbing, `app.rs` / `input.rs` key binding, and `ERR-TUI-01`
/// fall-back-to-previous-selection error handling are deferred to a
/// later task. This constant exists so future modules and tests have a
/// single symbol to point at when wiring model picker content.
pub const MODEL_PICKER_PLACEHOLDER: &str =
    "Model picker (placeholder — TuiEvent::SwitchModel + app.rs wiring + provider registry from TECH-AGS-PROVIDERS Phase 7 + ERR-TUI-01 fallback deferred)";

/// Local placeholder for a provider identifier.
///
/// Deliberately a `String` alias rather than the real provider-id type
/// that `TECH-AGS-PROVIDERS` Phase 7 will introduce. The full migration
/// will replace this alias with the canonical type and adjust call sites
/// accordingly.
pub type ProviderId = String;

/// Local placeholder for a model identifier.
///
/// Deliberately a `String` alias rather than the real model-id type that
/// `TECH-AGS-PROVIDERS` Phase 7 will introduce. The full migration will
/// replace this alias with the canonical type and adjust call sites
/// accordingly.
pub type ModelId = String;

/// A single (provider, model) entry shown in the model picker list.
///
/// The stub renderer does not yet display these entries — only an empty
/// bordered block is drawn — but they are stored on `ModelPickerState`
/// so tests and downstream callers can inspect the populated picker
/// today. The full migration will render these entries via a real
/// `List` widget driven by the provider registry.
#[derive(Debug, Clone, Default)]
pub struct ProviderEntry {
    /// Identifier of the provider this entry belongs to.
    pub provider_id: ProviderId,
    /// Identifier of the model offered by that provider.
    pub model_id: ModelId,
    /// Human-readable label shown in the picker list. The stub does not
    /// render it; the full migration will.
    pub label: String,
}

/// State for the model picker overlay.
///
/// Tracks the available provider/model entries, cursor position over
/// those entries, the most recently confirmed selection (if any), and an
/// optional error message reserved for the future `ERR-TUI-01` path. The
/// stub never populates `last_error` on its own — it exists so call
/// sites stay stable across the full migration.
#[derive(Debug, Default, Clone)]
pub struct ModelPickerState {
    /// All provider entries currently offered by the picker. Empty by
    /// default; the caller is expected to populate this before invoking
    /// `open`.
    pub providers: Vec<ProviderEntry>,
    /// Index of the currently highlighted entry. Always clamped to
    /// `0..providers.len()` (or `0` when `providers` is empty).
    pub cursor: usize,
    /// Index of the most recently confirmed selection, if any. Set by
    /// `Enter` in `on_key` and cleared on `Default`.
    pub selected_idx: Option<usize>,
    /// Optional error message reserved for the future `ERR-TUI-01`
    /// "provider unreachable" path. Always `None` in the stub unless a
    /// caller pre-populates it for testing; populated by the full
    /// migration when a switch attempt fails.
    pub last_error: Option<String>,
}

/// Open (or re-open) the model picker overlay.
///
/// Stub behaviour: clears any pre-existing `last_error` and resets
/// `cursor` to `0` so the highlight always lands on the top entry when
/// the picker becomes visible. Does NOT touch `providers` (the caller
/// owns that list) and does NOT touch `selected_idx` (so re-opening the
/// picker preserves the user's last confirmed selection).
///
/// The full migration will additionally refresh `providers` from the
/// real provider registry and surface any registry errors via
/// `last_error` (`ERR-TUI-01`).
pub fn open(state: &mut ModelPickerState) {
    state.cursor = 0;
    state.last_error = None;
}

/// Draw the model picker view into `area`.
///
/// Renders an empty bordered block titled "Model Picker" — actual list
/// rendering, error-line display for `ERR-TUI-01`, and provider/model
/// label formatting are deferred to the full migration. The stub
/// deliberately survives an empty `providers` list without panicking so
/// the picker can be drawn before any registry refresh has populated it.
///
/// NOTE: when `state.last_error` becomes populated (by the future
/// `ERR-TUI-01` path), the full migration will render its contents as a
/// status line at the bottom of this block. The stub leaves that
/// rendering to the migration; the field is read-only here.
pub fn draw(frame: &mut Frame, area: Rect, _state: &ModelPickerState) {
    let block = Block::default().borders(Borders::ALL).title("Model Picker");
    frame.render_widget(block, area);
}

/// Handle a key event for the model picker view.
///
/// Returns `false` because this scaffold does not yet consume input via
/// the parent app event loop in any meaningful way (the
/// `TuiEvent::SwitchModel` variant that would carry a confirmed
/// selection back to `app.rs` does not yet exist — Path A defers it).
/// Cursor movement is clamped to `0..providers.len()`:
///
/// * `Down` / `j` — advance cursor by one entry (clamped at the end)
/// * `Up`   / `k` — retreat cursor by one entry (saturating at zero)
/// * `Enter`      — set `selected_idx = Some(cursor)` IF there is at
///                  least one provider AND the cursor points inside the
///                  populated range; otherwise no-op
/// * `Esc`        — clear any `last_error` (the future migration will
///                  also dispatch a "close picker" event here)
///
/// All other keys are ignored. The full migration will replace this
/// with the real keymap dispatched through `input.rs`.
pub fn on_key(state: &mut ModelPickerState, key_code: KeyCode) -> bool {
    match key_code {
        KeyCode::Down | KeyCode::Char('j') => {
            let max_index = state.providers.len().saturating_sub(1);
            if state.cursor < max_index {
                state.cursor += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.cursor = state.cursor.saturating_sub(1);
        }
        KeyCode::Enter => {
            if state.cursor < state.providers.len() {
                state.selected_idx = Some(state.cursor);
            }
        }
        KeyCode::Esc => {
            state.last_error = None;
        }
        _ => {}
    }
    false
}

/// React to an agent event.
///
/// No-op stub. The full implementation will react to the eventual
/// `TuiEvent::SwitchModel { provider_id, model_id }` variant by
/// confirming or rejecting the switch and, on rejection, populating
/// `last_error` per `ERR-TUI-01`. That variant does NOT yet exist on
/// `crate::app::TuiEvent` — Path A explicitly defers adding it, along
/// with the `app.rs` `ModelPickerState` field and the `input.rs` key
/// binding.
pub fn on_agent_event(_state: &mut ModelPickerState) {}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::KeyCode;

    fn entry(provider: &str, model: &str) -> ProviderEntry {
        ProviderEntry {
            provider_id: provider.to_string(),
            model_id: model.to_string(),
            label: format!("{provider}/{model}"),
        }
    }

    fn populated_state() -> ModelPickerState {
        ModelPickerState {
            providers: vec![
                entry("anthropic", "claude-opus-4-6"),
                entry("anthropic", "claude-sonnet-4-5"),
                entry("openai", "gpt-5"),
            ],
            cursor: 0,
            selected_idx: None,
            last_error: None,
        }
    }

    #[test]
    fn model_picker_state_default_empty() {
        let state = ModelPickerState::default();
        assert!(state.providers.is_empty(), "default providers should be empty");
        assert_eq!(state.cursor, 0, "default cursor should be 0");
        assert!(
            state.selected_idx.is_none(),
            "default selected_idx should be None"
        );
        assert!(
            state.last_error.is_none(),
            "default last_error should be None"
        );
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
        state.cursor = state.providers.len() - 1;
        on_key(&mut state, KeyCode::Down);
        assert_eq!(
            state.cursor,
            state.providers.len() - 1,
            "Down at last entry should keep cursor clamped at providers.len()-1"
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
    fn on_key_enter_selects_current_cursor() {
        let mut state = populated_state();
        state.cursor = 2;
        on_key(&mut state, KeyCode::Enter);
        assert_eq!(
            state.selected_idx,
            Some(2),
            "Enter should set selected_idx to current cursor"
        );
    }

    #[test]
    fn on_key_enter_no_op_when_empty() {
        let mut state = ModelPickerState::default();
        on_key(&mut state, KeyCode::Enter);
        assert!(
            state.selected_idx.is_none(),
            "Enter on empty providers should not set selected_idx"
        );
    }

    #[test]
    fn on_key_esc_clears_last_error() {
        let mut state = populated_state();
        state.last_error = Some("provider unreachable".to_string());
        on_key(&mut state, KeyCode::Esc);
        assert!(
            state.last_error.is_none(),
            "Esc should clear last_error"
        );
    }

    #[test]
    fn open_resets_cursor_and_clears_error() {
        let mut state = populated_state();
        state.cursor = 2;
        state.last_error = Some("stale error".to_string());
        state.selected_idx = Some(1);

        open(&mut state);

        assert_eq!(state.cursor, 0, "open should reset cursor to 0");
        assert!(
            state.last_error.is_none(),
            "open should clear last_error"
        );
        assert_eq!(
            state.selected_idx,
            Some(1),
            "open should preserve previous selected_idx"
        );
        assert_eq!(
            state.providers.len(),
            3,
            "open should not touch providers list"
        );
    }

    #[test]
    fn draw_does_not_panic_empty() {
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("test backend terminal");
        let state = ModelPickerState::default();
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
