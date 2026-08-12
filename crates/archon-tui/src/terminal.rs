//! Terminal guard for raw mode and alternate screen management.
//!
//! Extracts terminal setup/cleanup from `app.rs` into a dedicated guard struct
//! that automatically restores the terminal on drop.
//!
//! # Teardown paths (issue #174)
//!
//! Entering pushes progressive keyboard-enhancement flags (see
//! [`keyboard`]) so `Shift+Enter` is distinguishable from `Enter`. Those
//! flags live on the *terminal*, not in this process: leaving them pushed
//! outlives archon and the user has no obvious way to undo it. Every way out
//! therefore funnels through [`restore_terminal`], which pops at most once:
//!
//! | path        | entry point                                  |
//! |-------------|----------------------------------------------|
//! | clean exit  | `<TerminalGuard as Drop>::drop`              |
//! | panic       | the hook installed by [`install_panic_restore_hook`] (and `src/panic_save.rs`, which calls [`restore_terminal`] directly) |
//! | suspend     | [`TerminalGuard::suspend`], undone by [`TerminalGuard::resume`] |
//!
//! `tests/keyboard_enhancement_teardown.rs` spawns a child process down each
//! of those paths and asserts the pop sequence reaches stdout.

use std::io::Result as IoResult;
use std::io::stdout;
use std::sync::Once;

pub mod keyboard;

pub use crate::events::TuiEvent;

/// Guard that manages raw mode and alternate screen lifecycle.
///
/// On creation via `enter()`, enables raw mode, enters the alternate screen,
/// hides the cursor, and pushes keyboard-enhancement flags when the terminal
/// supports them. On drop, restores the terminal to its original state (pops
/// the enhancement flags, shows cursor, leaves alternate screen, disables raw
/// mode).
pub struct TerminalGuard {
    /// Whether *this* guard turned raw mode on, so [`TerminalGuard::resume`]
    /// knows whether to turn it back on after a suspend.
    raw_mode: bool,
}

pub fn mouse_capture_enabled() -> bool {
    let explicit = std::env::var("ARCHON_TUI_MOUSE_CAPTURE").ok();
    mouse_capture_policy(explicit.as_deref(), running_under_wsl())
}

fn mouse_capture_policy(explicit: Option<&str>, is_wsl: bool) -> bool {
    match explicit
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("1" | "true" | "on" | "yes") => true,
        Some("0" | "false" | "off" | "no") => false,
        Some(_) => false,
        None => is_wsl,
    }
}

fn running_under_wsl() -> bool {
    std::env::var_os("WSL_INTEROP").is_some()
        || std::env::var_os("WSL_DISTRO_NAME").is_some()
        || std::fs::read_to_string("/proc/version")
            .map(|version| version.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false)
}

/// Enter the alternate screen, bracketed paste and cursor-hide modes.
///
/// Split out so `enter` (strict, production) and `enter_without_raw_mode`
/// (best-effort, teardown harness) drive exactly the same sequence.
fn enter_screen_modes() -> IoResult<()> {
    use crossterm::ExecutableCommand;
    use crossterm::cursor::Hide;
    use crossterm::event::EnableBracketedPaste;
    use crossterm::terminal::EnterAlternateScreen;

    stdout().execute(EnterAlternateScreen)?;
    stdout().execute(EnableBracketedPaste)?;
    stdout().execute(Hide)?;
    Ok(())
}

/// Restore the terminal to the state it had before [`TerminalGuard::enter`].
///
/// Best-effort and idempotent: it runs from `Drop`, from the panic hook, and
/// from [`TerminalGuard::suspend`], so it must tolerate being called twice
/// and must never panic. The keyboard-enhancement pop goes **first** —
/// terminals that scope the keyboard stack to the active screen buffer would
/// otherwise see the pop land on the primary screen's stack and leave the
/// alternate screen's entry (and hence the user's shell) in modified-keys
/// mode.
pub fn restore_terminal() {
    use crossterm::ExecutableCommand;
    use crossterm::cursor::Show;
    use crossterm::event::DisableBracketedPaste;
    use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};

    keyboard::deactivate();
    // Errors are discarded because there is nothing we can do about a failed
    // cleanup, and propagating one would panic at shutdown.
    let _ = stdout().execute(Show);
    let _ = stdout().execute(DisableBracketedPaste);
    let _ = stdout().execute(LeaveAlternateScreen);
    let _ = disable_raw_mode();
}

static PANIC_HOOK: Once = Once::new();

/// Install a process panic hook that restores the terminal before the
/// previous hook prints its message.
///
/// Idempotent — the first caller wins and later calls are no-ops, so this is
/// safe to call from every `TerminalGuard::enter`. `src/panic_save.rs`
/// installs its own hook later in startup and chains to this one; both call
/// [`restore_terminal`], which pops the enhancement flags only once.
pub fn install_panic_restore_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_terminal();
            previous(info);
        }));
    });
}

impl TerminalGuard {
    /// Enter raw mode and alternate screen, hiding the cursor.
    ///
    /// # Errors
    /// Returns an error if raw mode cannot be enabled or the alternate screen
    /// cannot be activated.
    pub fn enter() -> IoResult<Self> {
        use crossterm::terminal::enable_raw_mode;

        enable_raw_mode()?;
        enter_screen_modes()?;
        Ok(Self::finish_enter(true))
    }

    /// Everything [`TerminalGuard::enter`] does except raw mode.
    ///
    /// The teardown harness (`tests/keyboard_enhancement_teardown.rs`) runs
    /// child processes with stdout on a pipe, where `enable_raw_mode` has no
    /// terminal to act on. Every behaviour the harness observes — the
    /// enhancement push, the panic hook, suspend/resume, and the `Drop` pop —
    /// is the production code path, unchanged.
    #[doc(hidden)]
    pub fn enter_without_raw_mode() -> Self {
        let _ = enter_screen_modes();
        Self::finish_enter(false)
    }

    fn finish_enter(raw_mode: bool) -> Self {
        install_panic_restore_hook();
        keyboard::activate();
        Self { raw_mode }
    }

    /// Hand the terminal back before running another full-screen program (or
    /// stopping this one).
    ///
    /// Pops the enhancement flags and undoes the screen modes; pair with
    /// [`TerminalGuard::resume`]. A suspend that skipped the pop would leave
    /// the *other* program — and the shell after it — receiving disambiguated
    /// key encodings it never asked for.
    pub fn suspend(&self) {
        restore_terminal();
    }

    /// Re-take the terminal after [`TerminalGuard::suspend`].
    ///
    /// # Errors
    /// Returns an error if raw mode or the alternate screen cannot be
    /// re-entered.
    pub fn resume(&self) -> IoResult<()> {
        use crossterm::terminal::enable_raw_mode;

        if self.raw_mode {
            enable_raw_mode()?;
            enter_screen_modes()?;
        } else {
            let _ = enter_screen_modes();
        }
        keyboard::activate();
        Ok(())
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        restore_terminal();
    }
}

/// Install a SIGWINCH handler that sends resize events through the given channel.
///
/// SIGWINCH is raised by the terminal when its size changes. The handler
/// captures the new dimensions and sends a `TuiEvent::Resize` through the
/// channel for the TUI event loop to process.
///
/// # Arguments
/// * `tx` - Channel sender for TuiEvent messages
///
/// # Platform behaviour
/// On non-Unix platforms (e.g. Windows) SIGWINCH does not exist; this
/// function is a noop there. Windows surfaces terminal resize through
/// `crossterm::event::Event::Resize`, which the input loop already handles.
///
/// # Example
/// ```ignore
/// let (tx, rx) = archon_tui::event_channel::bounded_tui_event_channel();
/// install_sigwinch(tx);
/// ```
#[cfg(unix)]
pub fn install_sigwinch(tx: crate::event_channel::TuiEventSender) {
    crate::observability::spawn_named("tui-sigwinch-listener", async move {
        use tokio::signal::unix::Signal;
        use tokio::signal::unix::{SignalKind, signal};

        // SIGWINCH is not available on all Unix systems, but most notably
        // missing on macOS in some configurations. We handle the error gracefully.
        let mut sigwinch: Signal = match signal(SignalKind::window_change()) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("failed to register SIGWINCH handler: {e}");
                return;
            }
        };

        while let Some(()) = sigwinch.recv().await {
            // Get the new terminal size
            let (cols, rows) = crossterm::terminal::size().unwrap_or((80, 24));

            if tx.send(TuiEvent::Resize { cols, rows }).is_err() {
                // Receiver dropped - the TUI is shutting down
                break;
            }
        }
    });
}

/// Non-Unix noop variant of `install_sigwinch`.
///
/// SIGWINCH is a Unix-only signal; `tokio::signal::unix` does not exist on
/// Windows. Resize events on Windows arrive via `crossterm::event::Event::Resize`
/// from the input loop, so no signal handler is needed here.
#[cfg(not(unix))]
pub fn install_sigwinch(_tx: crate::event_channel::TuiEventSender) {
    tracing::debug!("install_sigwinch: skipped on non-Unix (Windows uses crossterm Resize event)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_guard_enter_produces_valid_guard() {
        // This test only validates that TerminalGuard can be created.
        // Actual terminal operations require a real TTY.
        // We use Result::is_ok to check the guard can be instantiated.
        let guard_result = TerminalGuard::enter();
        // If we're in a non-TTY environment, this may fail - that's ok
        if let Ok(guard) = guard_result {
            drop(guard);
        }
    }

    #[test]
    fn mouse_capture_defaults_to_wsl() {
        assert!(mouse_capture_policy(None, true));
        assert!(!mouse_capture_policy(None, false));
    }

    #[test]
    fn mouse_capture_env_overrides_wsl_detection() {
        assert!(!mouse_capture_policy(Some("0"), true));
        assert!(!mouse_capture_policy(Some("off"), true));
        assert!(mouse_capture_policy(Some("1"), false));
        assert!(mouse_capture_policy(Some("yes"), false));
    }

    /// `restore_terminal` is reached from three unrelated places and must
    /// never leave a pop owed, however many times it runs.
    #[test]
    fn restore_terminal_leaves_no_enhancement_pop_outstanding() {
        restore_terminal();
        assert!(!keyboard::is_active());
        restore_terminal();
        assert!(!keyboard::is_active());
    }
}
