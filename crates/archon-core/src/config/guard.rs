//! `[guard]` — advisories that nudge the model without ever vetoing it.
//!
//! A guard here observes what the agent is doing and, when a pattern is worth
//! naming, writes a message addressed to the model. It does not block a call,
//! rewrite one, or delay one. That distinction is the whole reason this is a
//! separate section from `[permissions]` and `[sandbox]`, which decide whether
//! something may happen at all.

use serde::{Deserialize, Serialize};

pub use archon_tools::repeat_tool_guard::RepeatToolConfig;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GuardConfig {
    /// `[guard.repeat_tool]` — consecutive identical tool calls (#200 Phase 2).
    pub repeat_tool: RepeatToolConfig,
}

impl GuardConfig {
    /// Reject a guard section that cannot mean what it says.
    pub fn validate(&self) -> Result<(), String> {
        self.repeat_tool.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_keys_deserialise_under_the_section_name() {
        let parsed: GuardConfig = toml::from_str(
            "[repeat_tool]\n\
             enabled = true\n\
             thresholds = [3, 5, 8]\n\
             exclude = [\"TodoWrite\"]\n\
             arguments_preview_chars = 500\n",
        )
        .expect("should parse");
        assert_eq!(parsed, GuardConfig::default());
    }

    #[test]
    fn a_non_default_section_round_trips() {
        let parsed: GuardConfig = toml::from_str(
            "[repeat_tool]\n\
             enabled = false\n\
             thresholds = [2, 4]\n\
             exclude = [\"TodoWrite\", \"Read\"]\n\
             arguments_preview_chars = 120\n",
        )
        .expect("should parse");
        assert!(!parsed.repeat_tool.enabled);
        assert_eq!(parsed.repeat_tool.thresholds, vec![2, 4]);
        assert_eq!(
            parsed.repeat_tool.exclude,
            vec!["TodoWrite".to_string(), "Read".to_string()]
        );
        assert_eq!(parsed.repeat_tool.arguments_preview_chars, 120);
    }

    /// An absent section must not turn the guard off.
    #[test]
    fn an_absent_section_keeps_the_defaults() {
        let parsed: GuardConfig = toml::from_str("").expect("should parse");
        assert!(parsed.repeat_tool.enabled);
        assert_eq!(parsed.repeat_tool.thresholds, vec![3, 5, 8]);
    }

    #[test]
    fn a_broken_threshold_list_fails_validation_rather_than_falling_back() {
        let parsed: GuardConfig =
            toml::from_str("[repeat_tool]\nthresholds = []\n").expect("should parse");
        assert!(parsed.validate().is_err());
    }
}
