/// Status bar data for the TUI bottom bar.
#[derive(Debug, Clone)]
pub struct StatusBar {
    pub model: String,
    pub identity_mode: String,
    pub permission_mode: String,
    pub cost: f64,
    pub git_branch: Option<String>,
}

impl Default for StatusBar {
    fn default() -> Self {
        Self {
            model: "claude-sonnet-4-6".into(),
            identity_mode: "spoof".into(),
            permission_mode: "ask".into(),
            cost: 0.0,
            git_branch: None,
        }
    }
}

impl StatusBar {
    /// Format the status bar text.
    pub fn format(&self) -> String {
        let mut parts = vec![
            self.model.clone(),
            self.identity_mode.clone(),
            self.permission_mode.clone(),
            format!("${:.2}", self.cost),
        ];

        if let Some(ref branch) = self.git_branch {
            parts.push(branch.clone());
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
}
