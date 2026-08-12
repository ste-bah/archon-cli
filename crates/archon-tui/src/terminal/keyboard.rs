//! Progressive keyboard-enhancement (kitty protocol) lifecycle.
//!
//! Without the kitty keyboard protocol most terminals — Windows Terminal
//! included — encode `Shift+Enter` byte-for-byte identically to `Enter`, so
//! the application receives a plain [`crossterm::event::KeyCode::Enter`] and
//! *cannot* tell the two apart. Pushing
//! [`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES`] asks the terminal
//! to report modifiers on keys that would otherwise collide (issue #174).
//!
//! The push is a *stack* operation on the terminal, not on our process: if
//! archon exits without popping, the terminal stays in modified-keys mode
//! after archon is gone and the user has no obvious way to undo it. That is a
//! worse bug than the one being fixed, so the pop is driven from a single
//! process-global latch ([`PUSHED`]) that every teardown path — clean exit,
//! panic hook, suspend — goes through, and which pops at most once.
//!
//! Sequences are rendered through [`crossterm::Command::write_ansi`] rather
//! than `execute!`/`queue!` on purpose: on Windows crossterm reports the two
//! enhancement commands as "not ANSI supported" and routes them to a
//! `execute_winapi` that returns `Unsupported`, which would make the teardown
//! harness untestable on Windows. `write_ansi` is crossterm's own definition
//! of the escape sequence, so we stay honest about the wire format while
//! remaining platform independent.

use std::io::{IsTerminal, Result as IoResult, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use crossterm::Command;
use crossterm::event::{
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};

/// Override for the terminal-support probe.
///
/// `1`/`true`/`on`/`yes` forces the push, `0`/`false`/`off`/`no` suppresses
/// it, anything else (or unset) falls back to
/// [`crossterm::terminal::supports_keyboard_enhancement`]. The force-on side
/// exists because crossterm hard-codes `Ok(false)` on Windows even though
/// Windows Terminal speaks the protocol in VT-input mode; the force-off side
/// is the escape hatch for a terminal that answers the probe but then
/// mis-encodes keys.
pub const ENHANCEMENT_ENV: &str = "ARCHON_TUI_KEYBOARD_ENHANCEMENT";

/// The single flag we ask for. `DISAMBIGUATE_ESCAPE_CODES` is the minimum
/// that separates `Shift+Enter` from `Enter`; the richer flags (event types,
/// alternate keys) would change the shape of every key event we already
/// handle and are deliberately not requested.
pub const FLAGS: KeyboardEnhancementFlags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES;

/// `true` between a successful push and its matching pop. Process-global
/// because the panic hook runs with no access to the [`super::TerminalGuard`].
static PUSHED: AtomicBool = AtomicBool::new(false);

fn ansi(command: impl Command) -> String {
    let mut rendered = String::new();
    // Writing into a String is infallible; crossterm's own `Command` impls
    // only ever return the `fmt::Result` of the underlying writer.
    let _ = command.write_ansi(&mut rendered);
    rendered
}

/// The escape sequence that pushes [`FLAGS`] onto the terminal's keyboard
/// stack (`CSI > 1 u`).
pub fn push_sequence() -> String {
    ansi(PushKeyboardEnhancementFlags(FLAGS))
}

/// The escape sequence that pops one entry off the terminal's keyboard stack
/// (`CSI < 1 u` — pop exactly one, not the whole stack).
pub fn pop_sequence() -> String {
    ansi(PopKeyboardEnhancementFlags)
}

fn parse_override(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

fn forced() -> Option<bool> {
    std::env::var(ENHANCEMENT_ENV)
        .ok()
        .as_deref()
        .and_then(parse_override)
}

/// Whether the enhancement should be pushed for this terminal.
///
/// The `is_terminal` guard is not cosmetic: crossterm's Unix probe writes a
/// query to `/dev/tty` (falling back to stdout) and then spins on
/// `poll_internal`, swallowing errors — with no controlling terminal that
/// loop never terminates. Never probe unless there is something to probe.
pub fn supported() -> bool {
    if let Some(forced) = forced() {
        return forced;
    }
    if !std::io::stdout().is_terminal() {
        return false;
    }
    crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false)
}

/// Push the flags into `out` when `enabled`.
///
/// Returns `true` when a push was written (and a pop is now owed). Pushing
/// while a push is already outstanding is a no-op, so repeated calls cannot
/// unbalance the terminal's stack.
fn push_into(out: &mut impl Write, enabled: bool) -> IoResult<bool> {
    if !enabled || PUSHED.load(Ordering::SeqCst) {
        return Ok(false);
    }
    out.write_all(push_sequence().as_bytes())?;
    out.flush()?;
    PUSHED.store(true, Ordering::SeqCst);
    Ok(true)
}

/// [`push_into`] gated on [`supported`].
pub fn activate_into(out: &mut impl Write) -> IoResult<bool> {
    push_into(out, supported())
}

/// Pop the flags from `out` if — and only if — a push is outstanding.
///
/// Returns `true` when a pop was written. The latch is cleared with a single
/// atomic `swap` so concurrent teardown paths (Drop racing the panic hook)
/// emit exactly one pop between them.
pub fn deactivate_into(out: &mut impl Write) -> IoResult<bool> {
    if !PUSHED.swap(false, Ordering::SeqCst) {
        return Ok(false);
    }
    out.write_all(pop_sequence().as_bytes())?;
    out.flush()?;
    Ok(true)
}

/// [`activate_into`] against the process stdout, swallowing I/O errors.
pub fn activate() -> bool {
    activate_into(&mut std::io::stdout()).unwrap_or(false)
}

/// [`deactivate_into`] against the process stdout, swallowing I/O errors.
///
/// Teardown runs from `Drop` and from the panic hook, where there is nothing
/// useful to do with an error and propagating one would abort the process.
pub fn deactivate() -> bool {
    deactivate_into(&mut std::io::stdout()).unwrap_or(false)
}

/// Whether a push is currently outstanding.
pub fn is_active() -> bool {
    PUSHED.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the wire format the teardown harness asserts on. If crossterm
    /// ever changes these bytes the harness must change with it, and this is
    /// where it fails first.
    #[test]
    fn sequences_match_the_kitty_protocol_wire_format() {
        assert_eq!(push_sequence(), "\u{1b}[>1u");
        assert_eq!(pop_sequence(), "\u{1b}[<1u");
    }

    #[test]
    fn disambiguate_is_the_only_requested_flag() {
        assert_eq!(FLAGS, KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES);
        assert_eq!(FLAGS.bits(), 1);
    }

    #[test]
    fn push_writes_nothing_when_the_terminal_does_not_support_it() {
        let mut sink: Vec<u8> = Vec::new();
        assert!(!push_into(&mut sink, false).expect("write to Vec cannot fail"));
        assert!(sink.is_empty());
        assert!(!is_active());
    }

    #[test]
    fn deactivate_is_a_noop_without_an_outstanding_push() {
        let mut sink: Vec<u8> = Vec::new();
        assert!(!deactivate_into(&mut sink).expect("write to Vec cannot fail"));
        assert!(sink.is_empty());
    }

    /// The latch is process-global, so this test owns it for its duration —
    /// it is the only unit test that pushes. The round trip proves a second
    /// push is suppressed (no double entry on the terminal's stack) and a
    /// second pop is suppressed (no popping an entry we never pushed).
    #[test]
    fn push_then_pop_is_balanced_and_idempotent_at_both_ends() {
        let mut sink: Vec<u8> = Vec::new();
        assert!(push_into(&mut sink, true).expect("write to Vec cannot fail"));
        assert!(is_active());
        assert!(!push_into(&mut sink, true).expect("write to Vec cannot fail"));
        assert_eq!(String::from_utf8(sink.clone()).unwrap(), push_sequence());

        sink.clear();
        assert!(deactivate_into(&mut sink).expect("write to Vec cannot fail"));
        assert!(!is_active());
        assert!(!deactivate_into(&mut sink).expect("write to Vec cannot fail"));
        assert_eq!(String::from_utf8(sink).unwrap(), pop_sequence());
    }

    #[test]
    fn env_override_parses_both_directions() {
        for raw in ["1", "true", "ON", " yes "] {
            assert_eq!(parse_override(raw), Some(true), "{raw:?}");
        }
        for raw in ["0", "false", "Off", "no"] {
            assert_eq!(parse_override(raw), Some(false), "{raw:?}");
        }
        assert_eq!(parse_override("maybe"), None);
        assert_eq!(parse_override(""), None);
    }
}
