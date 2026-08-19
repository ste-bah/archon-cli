//! Tool-output spill configuration (#189 Phase 1).

use serde::{Deserialize, Serialize};

/// How long spilled tool output is kept before a session start prunes it.
const DEFAULT_RETENTION_DAYS: u32 = 7;

/// Where oversized tool output goes so the omitted region stays retrievable.
///
/// Trimming a tool result is not the same as losing it. The model sees a
/// head/tail excerpt because the whole thing will not fit in the context
/// window, but the bytes it did not see are still worth something — without a
/// file to read, recovering the middle of a large result means running the
/// whole operation again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpillConfig {
    /// Write spill files at all.
    pub enabled: bool,
    /// Days a session's spill directory survives before being pruned.
    ///
    /// Pruning happens on session start rather than on a timer: a directory
    /// nobody has opened archon to look at is not urgent, and a background
    /// sweeper is one more thing that can delete the wrong path.
    pub retention_days: u32,
}

impl Default for SpillConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: DEFAULT_RETENTION_DAYS,
        }
    }
}

impl SpillConfig {
    /// Retention as a duration, or `None` when retention is disabled.
    ///
    /// Zero means "keep nothing", which would delete a file the moment it was
    /// written, so it is treated as "do not prune" instead — an accidental 0
    /// should not be the setting that silently defeats the feature.
    #[must_use]
    pub fn retention(self) -> Option<std::time::Duration> {
        (self.retention_days > 0)
            .then(|| std::time::Duration::from_secs(u64::from(self.retention_days) * 86_400))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spilling_is_on_by_default_for_a_week() {
        let config = SpillConfig::default();
        assert!(config.enabled);
        assert_eq!(config.retention_days, 7);
        assert_eq!(
            config.retention(),
            Some(std::time::Duration::from_secs(7 * 86_400))
        );
    }

    /// Zero must not mean "delete on write". A user who types 0 expecting
    /// "unlimited" gets unlimited, not a feature that silently does nothing.
    #[test]
    fn zero_retention_disables_pruning_rather_than_deleting_everything() {
        let config = SpillConfig {
            retention_days: 0,
            ..SpillConfig::default()
        };
        assert_eq!(config.retention(), None);
    }

    #[test]
    fn deserialises_from_the_documented_keys() {
        let parsed: SpillConfig =
            toml::from_str("enabled = false\nretention_days = 30").expect("should parse");
        assert!(!parsed.enabled);
        assert_eq!(parsed.retention_days, 30);
    }

    /// An empty section must not turn the feature off.
    #[test]
    fn an_empty_section_keeps_the_defaults() {
        let parsed: SpillConfig = toml::from_str("").expect("should parse");
        assert_eq!(parsed, SpillConfig::default());
    }
}
