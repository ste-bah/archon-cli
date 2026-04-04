// Personality profile loading from config.toml [personality] section.
// Phase 2 — TASK-CLI-105: REQ-CONSCIOUS-001

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A valid MBTI personality type (16 types).
const VALID_MBTI: &[&str] = &[
    "INTJ", "INTP", "ENTJ", "ENTP", "INFJ", "INFP", "ENFJ", "ENFP", "ISTJ", "ISFJ", "ESTJ",
    "ESFJ", "ISTP", "ISFP", "ESTP", "ESFP",
];

#[derive(Debug, thiserror::Error)]
pub enum PersonalityError {
    #[error("invalid MBTI type: \"{0}\". Valid types: INTJ, INTP, ENTJ, ENTP, INFJ, INFP, ENFJ, ENFP, ISTJ, ISFJ, ESTJ, ESFJ, ISTP, ISFP, ESTP, ESFP")]
    InvalidMbti(String),

    #[error("invalid enneagram: \"{0}\". Must match pattern [1-9]w[1-9] (e.g., 4w5)")]
    InvalidEnneagram(String),
}

/// Personality profile loaded from `[personality]` in config.toml.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PersonalityProfile {
    /// Name of the personality (e.g., "Archon").
    pub name: String,

    /// MBTI type (e.g., "INTJ"). Must be one of the 16 valid types.
    #[serde(rename = "type")]
    pub mbti_type: String,

    /// Enneagram type (e.g., "4w5"). Must match pattern [1-9]w[1-9].
    pub enneagram: String,

    /// Personality traits (e.g., ["strategic", "direct", "self-critical"]).
    pub traits: Vec<String>,

    /// Communication style (e.g., "terse").
    pub communication_style: String,
}

impl Default for PersonalityProfile {
    fn default() -> Self {
        Self {
            name: "Archon".into(),
            mbti_type: "INTJ".into(),
            enneagram: "4w5".into(),
            traits: vec![
                "strategic".into(),
                "direct".into(),
                "self-critical".into(),
                "truth-over-comfort".into(),
            ],
            communication_style: "terse".into(),
        }
    }
}

impl PersonalityProfile {
    /// Validate the profile. Returns `Ok(())` if valid, `Err` with details otherwise.
    pub fn validate(&self) -> Result<(), PersonalityError> {
        // Validate MBTI type (case-insensitive comparison)
        let upper = self.mbti_type.to_uppercase();
        if !VALID_MBTI.contains(&upper.as_str()) {
            return Err(PersonalityError::InvalidMbti(self.mbti_type.clone()));
        }

        // Validate enneagram pattern: [1-9]w[1-9]
        if !is_valid_enneagram(&self.enneagram) {
            return Err(PersonalityError::InvalidEnneagram(self.enneagram.clone()));
        }

        Ok(())
    }

    /// Format as prompt-injectable text block.
    pub fn to_prompt_text(&self) -> String {
        let traits_text = if self.traits.is_empty() {
            "none specified".to_string()
        } else {
            self.traits.join(", ")
        };

        format!(
            "Personality: {} ({}, {})\nTraits: {}\nStyle: {}",
            self.name, self.mbti_type, self.enneagram, traits_text, self.communication_style,
        )
    }
}

/// Check if a string matches the enneagram pattern `[1-9]w[1-9]`.
fn is_valid_enneagram(s: &str) -> bool {
    let bytes = s.as_bytes();
    bytes.len() == 3
        && bytes[0].is_ascii_digit()
        && bytes[0] != b'0'
        && bytes[1] == b'w'
        && bytes[2].is_ascii_digit()
        && bytes[2] != b'0'
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_valid() {
        let profile = PersonalityProfile::default();
        assert!(profile.validate().is_ok());
        assert_eq!(profile.name, "Archon");
        assert_eq!(profile.mbti_type, "INTJ");
        assert_eq!(profile.enneagram, "4w5");
    }

    #[test]
    fn all_16_mbti_types_accepted() {
        for mbti in VALID_MBTI {
            let profile = PersonalityProfile {
                mbti_type: mbti.to_string(),
                ..Default::default()
            };
            assert!(profile.validate().is_ok(), "MBTI type {mbti} should be valid");
        }
    }

    #[test]
    fn lowercase_mbti_accepted() {
        let profile = PersonalityProfile {
            mbti_type: "intj".into(),
            ..Default::default()
        };
        assert!(profile.validate().is_ok());
    }

    #[test]
    fn invalid_mbti_rejected() {
        let profile = PersonalityProfile {
            mbti_type: "XXXX".into(),
            ..Default::default()
        };
        let err = profile.validate().unwrap_err();
        assert!(matches!(err, PersonalityError::InvalidMbti(_)));
        assert!(err.to_string().contains("XXXX"));
    }

    #[test]
    fn empty_mbti_rejected() {
        let profile = PersonalityProfile {
            mbti_type: "".into(),
            ..Default::default()
        };
        assert!(profile.validate().is_err());
    }

    #[test]
    fn valid_enneagram_patterns() {
        for (core, wing) in [(1, 2), (4, 5), (9, 1), (7, 8)] {
            let profile = PersonalityProfile {
                enneagram: format!("{core}w{wing}"),
                ..Default::default()
            };
            assert!(
                profile.validate().is_ok(),
                "{}w{} should be valid",
                core,
                wing
            );
        }
    }

    #[test]
    fn invalid_enneagram_rejected() {
        for bad in ["0w5", "4w0", "10w5", "4x5", "4w", "w5", "", "45"] {
            let profile = PersonalityProfile {
                enneagram: bad.into(),
                ..Default::default()
            };
            assert!(
                profile.validate().is_err(),
                "Enneagram '{bad}' should be invalid"
            );
        }
    }

    #[test]
    fn format_prompt_text_includes_all_fields() {
        let profile = PersonalityProfile {
            name: "TestBot".into(),
            mbti_type: "ENFP".into(),
            enneagram: "7w8".into(),
            traits: vec!["curious".into(), "creative".into()],
            communication_style: "verbose".into(),
        };
        let text = profile.to_prompt_text();
        assert!(text.contains("TestBot"));
        assert!(text.contains("ENFP"));
        assert!(text.contains("7w8"));
        assert!(text.contains("curious, creative"));
        assert!(text.contains("verbose"));
    }

    #[test]
    fn format_prompt_text_empty_traits() {
        let profile = PersonalityProfile {
            traits: vec![],
            ..Default::default()
        };
        let text = profile.to_prompt_text();
        assert!(text.contains("none specified"));
    }

    #[test]
    fn deserialize_from_toml() {
        let toml_str = r#"
            name = "Archon"
            type = "INTJ"
            enneagram = "4w5"
            traits = ["strategic", "direct"]
            communication_style = "terse"
        "#;
        let profile: PersonalityProfile = toml::from_str(toml_str).unwrap();
        assert_eq!(profile.name, "Archon");
        assert_eq!(profile.mbti_type, "INTJ");
        assert_eq!(profile.traits.len(), 2);
    }

    #[test]
    fn deserialize_missing_fields_uses_defaults() {
        let toml_str = r#"
            name = "Custom"
        "#;
        let profile: PersonalityProfile = toml::from_str(toml_str).unwrap();
        assert_eq!(profile.name, "Custom");
        assert_eq!(profile.mbti_type, "INTJ"); // default
        assert_eq!(profile.enneagram, "4w5"); // default
    }

    #[test]
    fn deserialize_empty_toml_uses_all_defaults() {
        let toml_str = "";
        let profile: PersonalityProfile = toml::from_str(toml_str).unwrap();
        assert_eq!(profile, PersonalityProfile::default());
    }
}
