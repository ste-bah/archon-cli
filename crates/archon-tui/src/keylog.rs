//! `ARCHON_TUI_LOG_KEYS=1` input-wire instrumentation (issue #174).
//!
//! The input equivalent of the wire capture used for #75: when the env var is
//! set, every `crossterm::Event` the TUI loop receives and every mutation of
//! the input buffer is written to the tracing file, so an input defect can be
//! diagnosed from a trace instead of a screenshot. The trace is what
//! distinguishes "the terminal never sent us the modifier" from "we received
//! it and dropped it" — exactly the ambiguity that made Shift+Enter look like
//! a binding bug when it was a wire-encoding one.
//!
//! Zero cost when unset: the env var is read once into a [`OnceLock`] and
//! every entry point returns on a relaxed bool load before formatting
//! anything.

use std::sync::OnceLock;

use crossterm::event::{Event, KeyEvent};

/// Env var that turns the trace on. Any of `1`/`true`/`on`/`yes`.
pub const LOG_KEYS_ENV: &str = "ARCHON_TUI_LOG_KEYS";

/// Tracing target for every event this module emits, so a trace can be
/// filtered down with `RUST_LOG=archon_tui::keys=trace`.
const TARGET: &str = "archon_tui::keys";

static ENABLED: OnceLock<bool> = OnceLock::new();

fn parse_enabled(raw: Option<&str>) -> bool {
    matches!(
        raw.map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "on" | "yes")
    )
}

/// Whether key logging is on. Resolved once per process.
pub fn enabled() -> bool {
    *ENABLED.get_or_init(|| parse_enabled(std::env::var(LOG_KEYS_ENV).ok().as_deref()))
}

/// Log one raw terminal event as received from crossterm.
///
/// `Event::Key` is broken out field by field — kind and modifiers are the two
/// that input bugs hide in, and `{:?}` on the whole struct buries them.
pub fn log_event(event: &Event) {
    if !enabled() {
        return;
    }
    match event {
        Event::Key(key) => log_key(key),
        Event::Paste(text) => tracing::info!(
            target: TARGET,
            kind = "paste",
            bytes = text.len(),
            newlines = text.matches('\n').count(),
            raw = ?text,
            "bracketed paste",
        ),
        other => tracing::info!(target: TARGET, kind = "other", raw = ?other, "terminal event"),
    }
}

fn log_key(key: &KeyEvent) {
    tracing::info!(
        target: TARGET,
        kind = "key",
        event_kind = ?key.kind,
        code = ?key.code,
        modifiers = ?key.modifiers,
        state = ?key.state,
        raw = ?key,
        "key event",
    );
}

/// Log one mutation of the input buffer.
///
/// `op` names the call site (`insert`, `insert_newline`, `backspace`, …);
/// `text` and `cursor` are the buffer state *after* the mutation, so a trace
/// reads as a replayable sequence of buffer snapshots.
pub fn log_buffer(op: &str, text: &str, cursor: usize) {
    if !enabled() {
        return;
    }
    tracing::info!(
        target: TARGET,
        kind = "buffer",
        op,
        cursor,
        len = text.len(),
        newlines = text.matches('\n').count(),
        raw = ?text,
        "input buffer mutated",
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn env_values_that_enable_the_trace() {
        for raw in ["1", "true", "ON", " yes "] {
            assert!(parse_enabled(Some(raw)), "{raw:?}");
        }
    }

    #[test]
    fn env_values_that_leave_the_trace_off() {
        for raw in ["0", "false", "off", "no", "", "2"] {
            assert!(!parse_enabled(Some(raw)), "{raw:?}");
        }
        assert!(!parse_enabled(None));
    }

    /// The entry points must be safe to call with the trace off — that is the
    /// whole "zero cost when unset" contract, and it is the state every
    /// production run is in.
    #[test]
    fn entry_points_are_callable_regardless_of_state() {
        log_event(&Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT,
        )));
        log_event(&Event::Paste("a\nb".to_string()));
        log_event(&Event::Resize(80, 24));
        log_buffer("insert_newline", "a\nb", 3);
    }
}
