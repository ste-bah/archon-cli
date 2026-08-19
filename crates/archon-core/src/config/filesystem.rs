//! Freshness policy for writes to the working tree (#193 Phase A).

use serde::{Deserialize, Serialize};

/// What to do when a tool is about to write to a path the agent has not read,
/// or has read and something else has changed since.
///
/// `Edit` re-reads immediately before writing, so the bytes it replaces are
/// current. The risk is the *choice* of what to replace: `old_string` came from
/// a view of the file that may be arbitrarily old, and if the file moved on
/// since, the match can land somewhere that no longer means what the model
/// believed. The edit then succeeds, silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ReadBeforeEdit {
    /// Refuse the write and say why. The default: a wrong edit that reports
    /// success is worse than a refused one that explains itself.
    #[default]
    Block,
    /// Allow the write, and say so in the result.
    ///
    /// For a tree where something outside archon legitimately rewrites files
    /// mid-turn — a watcher, a formatter on save — and blocking would be noise.
    Warn,
    /// Do nothing. Restores the behaviour before this policy existed, exactly.
    Off,
}

/// `[filesystem]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct FilesystemConfig {
    /// Whether a write must be backed by a read of the same bytes.
    pub read_before_edit: ReadBeforeEdit,
}

impl FilesystemConfig {
    /// Whether the policy has anything to say at all.
    ///
    /// The whole check is skipped when it does not, so `off` costs nothing —
    /// which is what makes it a truthful "restores today's behaviour" rather
    /// than "runs the same code and ignores the answer".
    #[must_use]
    pub fn enforces_freshness(self) -> bool {
        self.read_before_edit != ReadBeforeEdit::Off
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Blocking is the default because the alternative is an edit that lands in
    /// the wrong place and reports success.
    #[test]
    fn the_default_blocks() {
        assert_eq!(
            FilesystemConfig::default().read_before_edit,
            ReadBeforeEdit::Block
        );
        assert!(FilesystemConfig::default().enforces_freshness());
    }

    #[test]
    fn off_disables_the_check_entirely() {
        let config = FilesystemConfig {
            read_before_edit: ReadBeforeEdit::Off,
        };
        assert!(!config.enforces_freshness());
    }

    #[test]
    fn the_three_settings_round_trip_through_toml() {
        for (text, expected) in [
            ("read_before_edit = \"block\"", ReadBeforeEdit::Block),
            ("read_before_edit = \"warn\"", ReadBeforeEdit::Warn),
            ("read_before_edit = \"off\"", ReadBeforeEdit::Off),
        ] {
            let parsed: FilesystemConfig = toml::from_str(text).expect(text);
            assert_eq!(parsed.read_before_edit, expected, "{text}");
        }
    }

    /// An absent section must not be a silently different policy from an
    /// explicit one.
    #[test]
    fn an_empty_section_is_the_default() {
        let parsed: FilesystemConfig = toml::from_str("").expect("empty");
        assert_eq!(parsed, FilesystemConfig::default());
    }
}
