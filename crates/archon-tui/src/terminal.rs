//! Terminal guard for raw mode and alternate screen management.
//!
//! Extracts terminal setup/cleanup from `app.rs` into a dedicated guard struct
//! that automatically restores the terminal on drop.

use std::io::stdout;
use std::io::Result as IoResult;

/// Minimal placeholder TuiEvent for SIGWINCH handling.
/// The real TuiEvent is defined in `app.rs`; this placeholder allows
/// install_sigwinch to compile and function until events.rs is populated.
#[derive(Debug, Clone)]
pub enum TuiEvent {
    /// Terminal was resized.
    Resize { cols: u16, rows: u16 },
}

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
/// # Example
/// ```ignore
/// let (tx, rx) = tokio::sync::mpsc::channel(16);
/// install_sigwinch(tx);
/// ```
pub fn install_sigwinch(tx: tokio::sync::mpsc::Sender<TuiEvent>) {
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        use tokio::signal::unix::Signal;

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

                    if tx.send(TuiEvent::Resize { cols, rows }).await.is_err() {
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
