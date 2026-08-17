//! Skill-system configuration (#187).

use serde::{Deserialize, Serialize};

/// What happens at turn end when a review has left unresolved gaps on the board.
///
/// A skill can describe a verification step, but it cannot stop a model from
/// declaring victory — prose is advice and the model is free to decide it has
/// done enough. The gate is the part that is not advice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CompletionGate {
    /// Refuse to end the turn and hand the findings back to the model to fix.
    ///
    /// The default, because the failure this exists to prevent is a turn that
    /// ends with "done" while a review it ran itself said otherwise. A warning
    /// would be read by a human after the fact; a block is read by the model
    /// while it can still act.
    #[default]
    Block,
    /// Let the turn end, but say what was still open.
    Warn,
    /// Do not check.
    Off,
}

impl CompletionGate {
    /// Parse from a config string, `None` for anything unrecognised.
    ///
    /// Callers warn and keep the default rather than failing the session — a
    /// typo in one field should not stop archon from starting.
    #[must_use]
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "block" => Some(Self::Block),
            "warn" => Some(Self::Warn),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Warn => "warn",
            Self::Off => "off",
        }
    }
}

/// Configuration for the skill system.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct SkillsConfig {
    /// Whether unresolved review gaps block the end of a turn.
    pub completion_gate: CompletionGate,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The default is the whole point of the feature. If this flips to `Warn`
    /// the gate becomes a log line nobody reads.
    #[test]
    fn the_gate_blocks_unless_told_otherwise() {
        assert_eq!(
            SkillsConfig::default().completion_gate,
            CompletionGate::Block
        );
    }

    #[test]
    fn parses_the_three_modes_case_insensitively() {
        assert_eq!(
            CompletionGate::from_str_opt("BLOCK"),
            Some(CompletionGate::Block)
        );
        assert_eq!(
            CompletionGate::from_str_opt(" warn "),
            Some(CompletionGate::Warn)
        );
        assert_eq!(
            CompletionGate::from_str_opt("off"),
            Some(CompletionGate::Off)
        );
    }

    #[test]
    fn an_unknown_mode_is_rejected_rather_than_guessed() {
        assert_eq!(CompletionGate::from_str_opt("blocking"), None);
        assert_eq!(CompletionGate::from_str_opt(""), None);
    }

    /// Round-trips through TOML under the name the docs give it.
    #[test]
    fn deserialises_from_the_documented_key() {
        let parsed: SkillsConfig =
            toml::from_str("completion_gate = \"warn\"").expect("should parse");
        assert_eq!(parsed.completion_gate, CompletionGate::Warn);

        let rendered = toml::to_string(&parsed).expect("should serialise");
        assert!(rendered.contains("completion_gate"), "{rendered}");
        assert!(rendered.contains("warn"), "{rendered}");
    }

    #[test]
    fn every_mode_round_trips_through_its_string() {
        for mode in [
            CompletionGate::Block,
            CompletionGate::Warn,
            CompletionGate::Off,
        ] {
            assert_eq!(CompletionGate::from_str_opt(mode.as_str()), Some(mode));
        }
    }
}
