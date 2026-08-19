#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextWarning {
    #[default]
    Ok,
    Warning,
    Critical,
}

impl ContextWarning {
    pub fn for_usage(tokens_used: u64, window: u64, compact_threshold: f32) -> Self {
        if window == 0 || tokens_used == 0 {
            return Self::Ok;
        }
        let fill = tokens_used as f32 / window as f32;
        if fill >= 0.95 {
            Self::Critical
        } else if fill >= (compact_threshold - 0.10).max(0.0) {
            Self::Warning
        } else {
            Self::Ok
        }
    }
}

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
    pub context_tokens_used: u64,
    pub context_window: u64,
    pub context_name: Option<String>,
    pub resolution_source: Option<String>,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub warning_state: ContextWarning,
    pub compact_threshold: f32,
    /// Tokens attributed to the largest single message (#189 Phase 3).
    ///
    /// Shown so the bar answers "what is filling the window", not only "how
    /// full is it" — the difference between knowing to compact and knowing
    /// what to drop. Zero until attribution has something to report.
    pub heaviest_message_tokens: u64,
    /// The whole ranking behind that one number (#192 scope B).
    ///
    /// The bar renders only `heaviest_message_tokens`; this is what `/context`
    /// opens the attribution overlay on. Kept here because it arrives on the
    /// same event and describes the same measurement — splitting them would
    /// let the bar and the overlay disagree about the same turn.
    pub token_attribution: TokenAttribution,
}

/// Per-message token attribution from the last context-pressure update.
///
/// `top_contributors` in `archon-core` has computed this since #189 Phase 3 and
/// had no caller outside its own tests; this is where it lands.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TokenAttribution {
    /// Ranked biggest-first, as `(message_index, tokens)`. Truncated by the
    /// emitter, so it is not the whole conversation.
    pub contributors: Vec<(usize, u64)>,
    /// Attributed tokens across *every* message, including those not listed.
    ///
    /// Without it a share cannot be computed from a truncated ranking, and a
    /// bare token count does not say whether it is most of the window.
    pub total: u64,
}

impl TokenAttribution {
    /// This contributor's share of the whole surface, 0.0–100.0.
    ///
    /// Zero when nothing has been attributed yet, rather than a division by
    /// zero dressed up as a percentage.
    #[must_use]
    pub fn share_percent(&self, tokens: u64) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (tokens as f64 / self.total as f64) * 100.0
    }
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
            context_tokens_used: 0,
            heaviest_message_tokens: 0,
            token_attribution: TokenAttribution::default(),
            context_window: 0,
            context_name: None,
            resolution_source: None,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            warning_state: ContextWarning::Ok,
            compact_threshold: 0.80,
        }
    }
}

impl StatusBar {
    pub fn update_context_warning(&mut self) {
        self.warning_state = ContextWarning::for_usage(
            self.context_tokens_used,
            self.context_window,
            self.compact_threshold,
        );
    }

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
        if self.context_window > 0 {
            let pct =
                (self.context_tokens_used as f64 / self.context_window as f64 * 100.0).min(100.0);
            let name = self
                .context_name
                .as_deref()
                .map(|n| format!("{n} "))
                .unwrap_or_default();
            let source = self
                .resolution_source
                .as_deref()
                .map(|s| format!(" {s}"))
                .unwrap_or_default();
            parts.push(format!(
                "ctx {name}{}k/{}k ({pct:.0}%{source})",
                self.context_tokens_used / 1000,
                self.context_window / 1000
            ));
        } else if self.context_tokens_used > 0 {
            parts.push(format!("ctx {}k/?", self.context_tokens_used / 1000));
        }
        // Only worth the width when one message is a real share of the window.
        // Below that the answer is "nothing in particular", and saying so every
        // turn is noise.
        if self.heaviest_message_tokens >= 1000 {
            let top = self.heaviest_message_tokens / 1000;
            // Name the command only when running it would show something. An
            // affordance that opens an empty box teaches the user to ignore it
            // (#192 scope B).
            parts.push(if self.token_attribution.contributors.is_empty() {
                format!("top {top}k")
            } else {
                format!("top {top}k /context")
            });
        }
        if self.cache_creation_tokens > 0 || self.cache_read_tokens > 0 {
            parts.push(format!(
                "cache {}/{}k",
                self.cache_creation_tokens / 1000,
                self.cache_read_tokens / 1000
            ));
        }

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

    #[test]
    fn context_warning_thresholds() {
        assert_eq!(
            ContextWarning::for_usage(699, 1000, 0.80),
            ContextWarning::Ok
        );
        assert_eq!(
            ContextWarning::for_usage(700, 1000, 0.80),
            ContextWarning::Warning
        );
        assert_eq!(
            ContextWarning::for_usage(950, 1000, 0.80),
            ContextWarning::Critical
        );
    }

    #[test]
    fn format_shows_context_window_before_usage() {
        let bar = StatusBar {
            context_tokens_used: 0,
            heaviest_message_tokens: 0,
            context_window: 1_000_000,
            context_name: Some("main".into()),
            resolution_source: Some("config".into()),
            ..Default::default()
        };
        assert!(bar.format().contains("ctx main 0k/1000k (0% config)"));
    }

    /// The bar has always said how full the window is. #189 Phase 3 makes it
    /// say what is filling it, which is the part that tells you what to drop.
    #[test]
    fn the_heaviest_message_is_shown_beside_the_context_total() {
        let bar = StatusBar {
            context_tokens_used: 500_000,
            heaviest_message_tokens: 92_000,
            context_window: 1_000_000,
            ..Default::default()
        };

        let rendered = bar.format();
        assert!(rendered.contains("ctx "), "{rendered}");
        assert!(rendered.contains("top 92k"), "{rendered}");
    }

    /// Below a thousand tokens the honest answer is "nothing in particular",
    /// and spending status-bar width to say that every turn is noise.
    #[test]
    fn a_small_heaviest_message_is_not_worth_the_width() {
        let bar = StatusBar {
            context_tokens_used: 5_000,
            heaviest_message_tokens: 400,
            context_window: 1_000_000,
            ..Default::default()
        };

        assert!(!bar.format().contains("top "), "{}", bar.format());
    }

    #[test]
    fn format_shows_cache_usage_when_present() {
        let bar = StatusBar {
            cache_creation_tokens: 1_000,
            cache_read_tokens: 2_000,
            ..Default::default()
        };
        assert!(bar.format().contains("cache 1/2k"));
    }
}
