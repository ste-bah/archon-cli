//! Model-free context pruning configuration (#189 Phase 8).

use serde::{Deserialize, Serialize};

/// Mechanical rules that reclaim context without asking a model.
///
/// Compaction always calls a model. Much of what fills a context window is
/// removable without judgement — a file read three times with no edit in
/// between, an error that was retried successfully a moment later, a tool
/// result whose full text is already on disk. Summarising those costs a
/// request and a wait to reach a conclusion that arithmetic reaches for free.
///
/// Each rule is toggleable on its own so a rule that misbehaves can be turned
/// off without reverting the feature or disabling the other two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PruneConfig {
    /// Run mechanical pruning before the model summarisation path.
    pub enabled: bool,
    /// Replace a spilled tool result's body with its locator once a later
    /// result supersedes it.
    pub spilled_superseded: bool,
    /// Collapse repeated reads of a file nothing has written to in between.
    pub repeated_reads: bool,
    /// Replace a failed tool result whose call succeeded on a later retry.
    pub retried_errors: bool,
}

impl Default for PruneConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            spilled_superseded: true,
            repeated_reads: true,
            retried_errors: true,
        }
    }
}

impl PruneConfig {
    /// Every rule off, but the pass still runs. Used by tests that want to
    /// assert the pass is inert rather than that it was skipped.
    #[must_use]
    pub fn no_rules() -> Self {
        Self {
            enabled: true,
            spilled_superseded: false,
            repeated_reads: false,
            retried_errors: false,
        }
    }

    /// Whether any rule could fire.
    #[must_use]
    pub fn any_rule_enabled(self) -> bool {
        self.spilled_superseded || self.repeated_reads || self.retried_errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pruning_and_every_rule_are_on_by_default() {
        let config = PruneConfig::default();
        assert!(config.enabled);
        assert!(config.spilled_superseded);
        assert!(config.repeated_reads);
        assert!(config.retried_errors);
    }

    /// A rule must be disableable on its own — that is the point of having
    /// three flags rather than one.
    #[test]
    fn one_rule_can_be_disabled_without_touching_the_others() {
        let parsed: PruneConfig = toml::from_str("repeated_reads = false").expect("should parse");

        assert!(!parsed.repeated_reads);
        assert!(parsed.spilled_superseded);
        assert!(parsed.retried_errors);
        assert!(parsed.enabled);
    }

    #[test]
    fn an_empty_section_keeps_the_defaults() {
        let parsed: PruneConfig = toml::from_str("").expect("should parse");
        assert_eq!(parsed, PruneConfig::default());
    }

    #[test]
    fn disabling_every_rule_is_distinguishable_from_disabling_the_pass() {
        assert!(PruneConfig::no_rules().enabled);
        assert!(!PruneConfig::no_rules().any_rule_enabled());
        assert!(PruneConfig::default().any_rule_enabled());
    }
}
