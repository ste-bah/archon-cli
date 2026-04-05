//! Terminal Panel for TASK-CLI-308.
//!
//! Delegates terminal emulation to **tmux** — no embedded PTY, no ANSI parsing.
//! When the user toggles the panel, the TUI is suspended and tmux runs in the
//! foreground. Pressing `Meta+J` inside tmux detaches back to Archon.
//!
//! Feature-gated: only compiled when `--features terminal-panel`.

use std::path::PathBuf;
use std::process::{Command, Stdio};

// ---------------------------------------------------------------------------
// TerminalPanelConfig
// ---------------------------------------------------------------------------

/// Configuration for a terminal panel instance.
#[derive(Debug, Clone)]
pub struct TerminalPanelConfig {
    session_id: String,
    cwd: PathBuf,
}

impl TerminalPanelConfig {
    /// Create a new config bound to `session_id` and working directory `cwd`.
    pub fn new(session_id: String, cwd: PathBuf) -> Self {
        Self { session_id, cwd }
    }

    /// Per-instance tmux socket name: `claude-panel-<first-8-chars-of-session-id>`.
    pub fn socket_name(&self) -> String {
        let prefix: String = self.session_id.chars().take(8).collect();
        format!("claude-panel-{}", prefix)
    }

    /// Args to create a new detached tmux session.
    ///
    /// `tmux -L <socket> new-session -d -s panel -c <cwd> <shell> -l`
    pub fn tmux_new_session_args(&self, shell: &str) -> Vec<String> {
        vec![
            "-L".into(),
            self.socket_name(),
            "new-session".into(),
            "-d".into(),
            "-s".into(),
            TerminalPanel::SESSION_NAME.into(),
            "-c".into(),
            self.cwd.to_string_lossy().into_owned(),
            shell.into(),
            "-l".into(),
        ]
    }

    /// Args to attach to the existing panel session.
    ///
    /// `tmux -L <socket> attach-session -t panel`
    pub fn tmux_attach_args(&self) -> Vec<String> {
        vec![
            "-L".into(),
            self.socket_name(),
            "attach-session".into(),
            "-t".into(),
            TerminalPanel::SESSION_NAME.into(),
        ]
    }

    /// Args to kill the tmux server for this socket (called on Archon exit).
    ///
    /// `tmux -L <socket> kill-server`
    pub fn tmux_kill_server_args(&self) -> Vec<String> {
        vec!["-L".into(), self.socket_name(), "kill-server".into()]
    }

    /// Args to bind `Meta+J` (`M-j`) → `detach-client` inside tmux.
    ///
    /// `tmux -L <socket> bind-key -n M-j detach-client`
    pub fn tmux_bind_metaj_args(&self) -> Vec<String> {
        vec![
            "-L".into(),
            self.socket_name(),
            "bind-key".into(),
            "-n".into(),
            "M-j".into(),
            "detach-client".into(),
        ]
    }

    /// Args to set the tmux status bar right-side message.
    ///
    /// `tmux -L <socket> set-option -g status-right "Alt+J to return to Claude"`
    pub fn tmux_status_hint_args(&self) -> Vec<String> {
        vec![
            "-L".into(),
            self.socket_name(),
            "set-option".into(),
            "-g".into(),
            "status-right".into(),
            "Alt+J to return to Claude".into(),
        ]
    }
}

// ---------------------------------------------------------------------------
// TerminalPanel
// ---------------------------------------------------------------------------

/// Manages the terminal panel lifecycle via tmux.
///
/// Singleton per Archon session — create once, reuse across toggles.
pub struct TerminalPanel {
    config: TerminalPanelConfig,
    open: bool,
    initialized: bool,
}

impl TerminalPanel {
    /// Constant tmux session name used for the panel.
    pub const SESSION_NAME: &'static str = "panel";

    /// Create a new (closed, uninitialized) terminal panel.
    pub fn new(config: TerminalPanelConfig) -> Self {
        Self {
            config,
            open: false,
            initialized: false,
        }
    }

    /// Returns true if the panel is currently showing (tmux in foreground).
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Returns true if the tmux session has been created at least once.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Detect the user's preferred shell.
    ///
    /// Uses `$SHELL` env var; falls back to `/bin/bash`.
    pub fn detect_shell() -> String {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }

    /// Check whether `tmux` is available on `$PATH`.
    ///
    /// Result is not cached here — callers who need caching should wrap externally.
    pub fn is_tmux_available() -> bool {
        Command::new("tmux")
            .arg("-V")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Toggle the terminal panel open/closed.
    ///
    /// - If tmux is available: creates session on first toggle, attaches thereafter.
    /// - If tmux is unavailable: spawns `$SHELL -i -l` directly (non-persistent).
    ///
    /// This call **blocks** while the user is in the terminal. The caller is
    /// expected to have already suspended the ratatui TUI (restored terminal state).
    /// After this returns, the caller should resume the TUI.
    pub fn toggle(&mut self) -> std::io::Result<()> {
        if self.open {
            // Already open — this is a no-op in the toggle sense; the panel
            // is closed by the user pressing Meta+J inside tmux, which causes
            // `attach-session` to return. We just update state.
            self.open = false;
            return Ok(());
        }

        let shell = Self::detect_shell();

        if Self::is_tmux_available() {
            self.toggle_with_tmux(&shell)
        } else {
            self.toggle_with_shell_fallback(&shell)
        }
    }

    /// Schedule async cleanup on Archon exit.
    ///
    /// Spawns `tmux -L <socket> kill-server` as a detached process (fire-and-forget).
    pub fn schedule_cleanup(&self) {
        let args = self.config.tmux_kill_server_args();
        let _ = Command::new("tmux")
            .args(&args[..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    fn toggle_with_tmux(&mut self, shell: &str) -> std::io::Result<()> {
        if !self.initialized {
            // Check if session already exists (e.g., from previous Archon crash)
            let session_exists = self.tmux_session_exists();

            if !session_exists {
                // Create new detached tmux session
                let new_args = self.config.tmux_new_session_args(shell);
                let status = Command::new("tmux").args(&new_args[..]).status()?;

                if !status.success() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "tmux new-session failed",
                    ));
                }

                // Bind Meta+J → detach-client
                let bind_args = self.config.tmux_bind_metaj_args();
                let _ = Command::new("tmux").args(&bind_args[..]).status();

                // Set status bar hint
                let hint_args = self.config.tmux_status_hint_args();
                let _ = Command::new("tmux").args(&hint_args[..]).status();
            }

            self.initialized = true;
        }

        // Attach — blocks until user presses Meta+J (detach-client)
        let attach_args = self.config.tmux_attach_args();
        self.open = true;
        let _ = Command::new("tmux").args(&attach_args[..]).status()?;
        self.open = false;
        Ok(())
    }

    fn toggle_with_shell_fallback(&mut self, shell: &str) -> std::io::Result<()> {
        self.open = true;
        let _ = Command::new(shell).args(["-i", "-l"]).status()?;
        self.open = false;
        Ok(())
    }

    fn tmux_session_exists(&self) -> bool {
        // `tmux -L <socket> has-session -t panel` exits 0 if session exists
        let socket = self.config.socket_name();
        Command::new("tmux")
            .args(["-L", &socket, "has-session", "-t", Self::SESSION_NAME])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}
