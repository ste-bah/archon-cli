//! The TUI entry points, split out of `app.rs` for the 500-line file-size
//! gate.
//!
//! Terminal setup, the backend-injection seams the integration tests use, and
//! nothing about application state. Re-exported from `app` so every existing
//! `archon_tui::app::run` path still resolves.

use std::io;

use crossterm::ExecutableCommand;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::app::AppConfig;
use crate::terminal::TerminalGuard;

/// Thin entry point that sets up terminal infrastructure and delegates to
/// [`crate::event_loop::run_inner`]. The public API called from `main.rs`.
pub async fn run(config: AppConfig) -> Result<(), io::Error> {
    // Setup terminal - TerminalGuard handles raw mode, alternate screen, and cursor hide.
    // Its Drop will restore the terminal on function exit.
    let _guard = TerminalGuard::enter()?;
    // Keep normal terminal text selection available by default, but auto-capture
    // on WSL because alternate-screen scrollback is unreliable there.
    let mouse_capture = crate::terminal::mouse_capture_enabled();
    if mouse_capture {
        io::stdout().execute(EnableMouseCapture)?;
    }
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // TASK-TUI-406: spawn BACKGROUND_AGENTS GC janitor at startup (60s
    // interval). Detached — task runs for TUI session lifetime.
    // Accessed via archon_core's re-export (archon-tools is dev-only dep).
    let _gc_handle = archon_core::background_agents::spawn_gc_task();

    let result = crate::event_loop::run_inner(config, &mut terminal).await;

    // Restore terminal - TerminalGuard's Drop handles cursor show, leave
    // alternate screen, bracketed paste, and raw mode.
    if mouse_capture {
        io::stdout().execute(DisableMouseCapture)?;
    }

    result
}

/// Backend-injection seam for integration tests (TUI-327).
pub async fn run_with_backend<B>(
    config: AppConfig,
    terminal: &mut ratatui::Terminal<B>,
) -> Result<(), io::Error>
where
    B: ratatui::backend::Backend,
{
    crate::event_loop::run_inner(config, terminal).await
}

/// Headless backend-injection seam for tests that use `TestBackend` and have
/// no crossterm terminal-event source.
pub async fn run_with_backend_without_terminal_events<B>(
    config: AppConfig,
    terminal: &mut ratatui::Terminal<B>,
) -> Result<(), io::Error>
where
    B: ratatui::backend::Backend,
{
    crate::event_loop::run_inner_without_terminal_events(config, terminal).await
}
