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
    /// Current voice input state for display in the status bar.
    pub voice_state: Option<crate::voice::VoiceState>,
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
            voice_state: None,
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

        // Show [brief] indicator when not in verbose mode (CLI-314)
        if !self.verbose {
            parts.push("[brief]".to_owned());
        }

        // Show voice state indicator when active
        match &self.voice_state {
            Some(crate::voice::VoiceState::Listening) => parts.push("[listening]".to_owned()),
            Some(crate::voice::VoiceState::Transcribing) => parts.push("[transcribing]".to_owned()),
            _ => {}
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
