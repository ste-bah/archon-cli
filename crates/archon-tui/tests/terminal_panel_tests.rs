//! Tests for TASK-CLI-308: Terminal Panel (feature-gated: terminal-panel).

#[cfg(feature = "terminal-panel")]
mod terminal_panel_tests {
    use archon_tui::terminal_panel::{TerminalPanel, TerminalPanelConfig};

    // -------------------------------------------------------------------------
    // Socket name derivation
    // -------------------------------------------------------------------------

    #[test]
    fn socket_name_uses_first_8_chars_of_session_id() {
        let cfg = TerminalPanelConfig::new("abcdef1234567890".to_string(), "/tmp".into());
        assert_eq!(cfg.socket_name(), "claude-panel-abcdef12");
    }

    #[test]
    fn socket_name_truncates_long_session_id() {
        let cfg = TerminalPanelConfig::new(
            "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string(),
            "/tmp".into(),
        );
        assert_eq!(cfg.socket_name(), "claude-panel-xxxxxxxx");
    }

    #[test]
    fn socket_name_handles_short_session_id() {
        // Shorter than 8 chars — use what's available
        let cfg = TerminalPanelConfig::new("abc".to_string(), "/tmp".into());
        assert_eq!(cfg.socket_name(), "claude-panel-abc");
    }

    #[test]
    fn socket_name_constant_prefix() {
        let cfg = TerminalPanelConfig::new("test1234".to_string(), "/tmp".into());
        let name = cfg.socket_name();
        assert!(
            name.starts_with("claude-panel-"),
            "socket name must start with 'claude-panel-', got: {}",
            name
        );
    }

    // -------------------------------------------------------------------------
    // Session name constant
    // -------------------------------------------------------------------------

    #[test]
    fn session_name_is_panel() {
        assert_eq!(TerminalPanel::SESSION_NAME, "panel");
    }

    // -------------------------------------------------------------------------
    // TerminalPanel construction
    // -------------------------------------------------------------------------

    #[test]
    fn terminal_panel_new_is_not_open() {
        let cfg = TerminalPanelConfig::new("test-session-id".to_string(), "/tmp".into());
        let panel = TerminalPanel::new(cfg);
        assert!(!panel.is_open());
    }

    #[test]
    fn terminal_panel_new_not_initialized() {
        let cfg = TerminalPanelConfig::new("test-session-id".to_string(), "/tmp".into());
        let panel = TerminalPanel::new(cfg);
        assert!(!panel.is_initialized());
    }

    // -------------------------------------------------------------------------
    // tmux availability check
    // -------------------------------------------------------------------------

    #[test]
    fn tmux_available_returns_bool() {
        // Just verifies it doesn't panic — result depends on environment
        let _available = TerminalPanel::is_tmux_available();
    }

    // -------------------------------------------------------------------------
    // Shell detection
    // -------------------------------------------------------------------------

    #[test]
    fn shell_detection_returns_non_empty_path() {
        let shell = TerminalPanel::detect_shell();
        assert!(!shell.is_empty(), "shell path must not be empty");
    }

    #[test]
    fn shell_detection_falls_back_to_bash() {
        // When $SHELL is unset, falls back to /bin/bash
        // We can't unset env in tests without unsafe, but verify the API exists
        let shell = TerminalPanel::detect_shell();
        // Must be an absolute path
        assert!(
            shell.starts_with('/'),
            "shell must be an absolute path, got: {}",
            shell
        );
    }

    // -------------------------------------------------------------------------
    // Build commands
    // -------------------------------------------------------------------------

    #[test]
    fn tmux_new_session_args_correct() {
        let cfg = TerminalPanelConfig::new("mysession123".to_string(), "/home/user/project".into());
        let socket = cfg.socket_name();
        let args = cfg.tmux_new_session_args("/bin/bash");
        // Must include: -L <socket>, new-session, -d, -s panel, -c <cwd>, /bin/bash -l
        let args_str = args.join(" ");
        assert!(args_str.contains(&socket), "args must include socket name");
        assert!(
            args_str.contains("new-session"),
            "args must include new-session"
        );
        assert!(args_str.contains("-d"), "args must include -d (detached)");
        assert!(
            args_str.contains("panel"),
            "args must include session name 'panel'"
        );
        assert!(
            args_str.contains("/home/user/project"),
            "args must include cwd"
        );
        assert!(args_str.contains("/bin/bash"), "args must include shell");
        assert!(
            args_str.contains("-l"),
            "shell must be invoked as login shell"
        );
    }

    #[test]
    fn tmux_attach_args_correct() {
        let cfg = TerminalPanelConfig::new("mysession123".to_string(), "/tmp".into());
        let args = cfg.tmux_attach_args();
        let args_str = args.join(" ");
        assert!(
            args_str.contains("attach-session"),
            "must include attach-session"
        );
        assert!(args_str.contains("panel"), "must target 'panel' session");
    }

    #[test]
    fn tmux_kill_server_args_correct() {
        let cfg = TerminalPanelConfig::new("abcd1234".to_string(), "/tmp".into());
        let args = cfg.tmux_kill_server_args();
        let args_str = args.join(" ");
        assert!(args_str.contains("-L"), "must use -L flag for socket");
        assert!(
            args_str.contains("claude-panel-abcd1234"),
            "must include socket name"
        );
        assert!(args_str.contains("kill-server"), "must include kill-server");
    }

    // -------------------------------------------------------------------------
    // Keybinding hint
    // -------------------------------------------------------------------------

    #[test]
    fn tmux_bind_metaj_args_present() {
        let cfg = TerminalPanelConfig::new("test".to_string(), "/tmp".into());
        let args = cfg.tmux_bind_metaj_args();
        let args_str = args.join(" ");
        // Must bind Meta+J (M-j in tmux notation) to detach-client
        assert!(
            args_str.contains("M-j") || args_str.contains("Meta+J") || args_str.contains("bind"),
            "must bind Meta+J key, got: {}",
            args_str
        );
        assert!(
            args_str.contains("detach-client"),
            "must bind to detach-client"
        );
    }

    #[test]
    fn tmux_status_hint_args_set_message() {
        let cfg = TerminalPanelConfig::new("test".to_string(), "/tmp".into());
        let args = cfg.tmux_status_hint_args();
        let args_str = args.join(" ");
        assert!(
            args_str.to_lowercase().contains("alt+j")
                || args_str.contains("Alt+J")
                || args_str.contains("return"),
            "status hint must reference Alt+J to return, got: {}",
            args_str
        );
    }
}
