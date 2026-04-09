/// Status bar data for the TUI bottom bar.
#[derive(Debug, Clone)]
pub struct StatusBar {
    pub model: String,
    pub identity_mode: String,
    pub permission_mode: String,
    pub cost: f64,
    pub git_branch: Option<String>,
    /// Current verbosity mode. `true` = verbose (default), `false` = brief.
    pub verbose: bool,
    /// Active agent name (shown when running with --agent).
    pub agent_name: Option<String>,
    /// Agent display color (hex or named color, used by TUI renderer).
    pub agent_color: Option<String>,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".into(),
            identity_mode: "spoof".into(),
            permission_mode: "ask".into(),
            cost: 0.0,
            git_branch: None,
            verbose: true,
            agent_name: None,
            agent_color: None,
        }
    }
}

impl StatusBar {
    /// Format the status bar text.
    pub fn format(&self) -> String {
        let mut parts = Vec::new();

        // Show active agent name first when in --agent mode
        if let Some(ref name) = self.agent_name {
            parts.push(format!("[{name}]"));
        }

        parts.push(self.model.clone());
        parts.push(self.identity_mode.clone());
        parts.push(self.permission_mode.clone());
        parts.push(format!("${:.2}", self.cost));

        if let Some(ref branch) = self.git_branch {
            parts.push(branch.clone());
        }

        // Show [brief] indicator when not in verbose mode (CLI-314)
        if !self.verbose {
            parts.push("[brief]".to_owned());
        }

        parts.join(" | ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_status_bar() {
        let bar = StatusBar::default();
        let text = bar.format();
        assert!(text.contains("claude-sonnet-4-6"));
        assert!(text.contains("spoof"));
        assert!(text.contains("$0.00"));
    }

    #[test]
    fn format_with_git_branch() {
        let bar = StatusBar {
            git_branch: Some("main".into()),
            ..Default::default()
        };
        let text = bar.format();
        assert!(text.contains("main"));
    }

    #[test]
    fn format_with_agent_name() {
        let bar = StatusBar {
            agent_name: Some("code-reviewer".into()),
            agent_color: Some("#ff0000".into()),
            ..Default::default()
        };
        let text = bar.format();
        assert!(text.starts_with("[code-reviewer]"));
        assert!(text.contains("claude-sonnet-4-6"));
    }

    #[test]
    fn format_without_agent_has_no_brackets() {
        let bar = StatusBar::default();
        let text = bar.format();
        assert!(!text.contains('[') || text.contains("[brief]"));
    }
}
