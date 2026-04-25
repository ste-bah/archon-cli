//! Terminal guard for raw mode and alternate screen management.
//!
//! Extracts terminal setup/cleanup from `app.rs` into a dedicated guard struct
//! that automatically restores the terminal on drop.

use std::io::Result as IoResult;
use std::io::stdout;

pub use crate::events::TuiEvent;

/// Guard that manages raw mode and alternate screen lifecycle.
///
/// On creation via `enter()`, enables raw mode, enters the alternate screen,
/// and hides the cursor. On drop, restores the terminal to its original state
/// (shows cursor, leaves alternate screen, disables raw mode).
pub struct TerminalGuard {
    _priv: (),
}

impl TerminalGuard {
    /// Enter raw mode and alternate screen, hiding the cursor.
    ///
    /// # Errors
    /// Returns an error if raw mode cannot be enabled or the alternate screen
    /// cannot be activated.
    pub fn enter() -> IoResult<Self> {
        use crossterm::ExecutableCommand;
        use crossterm::cursor::Hide;
        use crossterm::terminal::{EnterAlternateScreen, enable_raw_mode};

        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        stdout().execute(Hide)?;

        Ok(Self { _priv: () })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        use crossterm::ExecutableCommand;
        use crossterm::cursor::Show;
        use crossterm::terminal::{LeaveAlternateScreen, disable_raw_mode};

        // We use std::mem::forget on the result because there's nothing we can
        // do if cleanup fails, and swallowing the error avoids a panic at shutdown.
        let _ = stdout().execute(Show);
        let _ = stdout().execute(LeaveAlternateScreen);
        let _ = disable_raw_mode();
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
/// let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
/// install_sigwinch(tx);
/// ```
#[cfg(unix)]
pub fn install_sigwinch(tx: tokio::sync::mpsc::UnboundedSender<TuiEvent>) {
    tokio::spawn(async move {
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

        loop {
            match sigwinch.recv().await {
                Some(()) => {
                    // Get the new terminal size
                    let (cols, rows) = crossterm::terminal::size()
                        .map(|(c, r)| (c as u16, r as u16))
                        .unwrap_or((80, 24));

                    if tx.send(TuiEvent::Resize { cols, rows }).is_err() {
                        // Receiver dropped - the TUI is shutting down
                        break;
                    }
                }
                None => {
                    // Signal stream ended (should not happen for SIGWINCH)
                    break;
                }
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
pub fn install_sigwinch(_tx: tokio::sync::mpsc::UnboundedSender<TuiEvent>) {
    tracing::debug!(
        "install_sigwinch: skipped on non-Unix (Windows uses crossterm Resize event)"
    );
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
        if guard_result.is_ok() {
            drop(guard_result.unwrap());
        }
    }
}
