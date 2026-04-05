use serde_json::json;
use std::fmt;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const EFFORT_BETA: &str = "effort-2025-11-24";

// ---------------------------------------------------------------------------
// Effort level enum
// ---------------------------------------------------------------------------

/// Controls the reasoning effort the model should apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortLevel {
    High,
    Medium,
    Low,
}

impl fmt::Display for EffortLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        };
        f.write_str(s)
    }
}

impl FromStr for EffortLevel {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_level(s)
    }
}

// ---------------------------------------------------------------------------
// Effort state
// ---------------------------------------------------------------------------

/// Tracks the current effort level for API requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortState {
    level: EffortLevel,
}

impl Default for EffortState {
    fn default() -> Self {
        Self {
            level: EffortLevel::Medium,
        }
    }
}

impl EffortState {
    /// Create a new `EffortState` at the default (`Medium`) level.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the effort level.
    pub fn set_level(&mut self, level: EffortLevel) {
        self.level = level;
    }

    /// Current effort level.
    pub fn level(&self) -> EffortLevel {
        self.level
    }

    /// Build the JSON effort parameter for non-default levels.
    ///
    /// Returns `{"effort": "<level>"}` for `Medium` and `Low`, `None` for
    /// `High` (the default, which needs no parameter).
    pub fn effort_param(&self) -> Option<serde_json::Value> {
        match self.level {
            EffortLevel::High => None,
            other => Some(json!({ "effort": other.to_string() })),
        }
    }

    /// Return the beta header string required for non-default effort levels.
    ///
    /// Returns `Some("effort-2025-11-24")` for `Medium`/`Low`, `None` for `High`.
    pub fn beta_header(&self) -> Option<&'static str> {
        match self.level {
            EffortLevel::High => None,
            _ => Some(EFFORT_BETA),
        }
    }
}

// ---------------------------------------------------------------------------
// Public helper
// ---------------------------------------------------------------------------

/// Parse a case-insensitive string into an [`EffortLevel`].
pub fn parse_level(s: &str) -> Result<EffortLevel, String> {
    match s.trim().to_lowercase().as_str() {
        "high" => Ok(EffortLevel::High),
        "medium" | "med" => Ok(EffortLevel::Medium),
        "low" => Ok(EffortLevel::Low),
        _ => Err(format!(
            "invalid effort level: '{s}' (expected high, medium, or low)"
        )),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- EffortLevel display & parse ------------------------------------------

    #[test]
    fn display_all_levels() {
        assert_eq!(EffortLevel::High.to_string(), "high");
        assert_eq!(EffortLevel::Medium.to_string(), "medium");
        assert_eq!(EffortLevel::Low.to_string(), "low");
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(parse_level("HIGH"), Ok(EffortLevel::High));
        assert_eq!(parse_level("Medium"), Ok(EffortLevel::Medium));
        assert_eq!(parse_level("low"), Ok(EffortLevel::Low));
        assert_eq!(parse_level(" MED "), Ok(EffortLevel::Medium));
    }

    #[test]
    fn parse_invalid_returns_error() {
        let result = parse_level("turbo");
        assert!(result.is_err());
        assert!(result.as_ref().err().is_some_and(|e| e.contains("turbo")));
    }

    #[test]
    fn from_str_trait() {
        let level: EffortLevel = "low".parse().expect("should parse");
        assert_eq!(level, EffortLevel::Low);
    }

    // -- EffortState ----------------------------------------------------------

    #[test]
    fn default_is_medium() {
        let state = EffortState::new();
        assert_eq!(state.level(), EffortLevel::Medium);
    }

    #[test]
    fn set_level_updates() {
        let mut state = EffortState::new();
        state.set_level(EffortLevel::Low);
        assert_eq!(state.level(), EffortLevel::Low);
    }

    // -- effort_param ---------------------------------------------------------

    #[test]
    fn param_none_for_high() {
        let mut state = EffortState::new();
        state.set_level(EffortLevel::High);
        assert!(state.effort_param().is_none());
    }

    #[test]
    fn param_medium_json() {
        let mut state = EffortState::new();
        state.set_level(EffortLevel::Medium);
        let val = state.effort_param().expect("should be Some for medium");
        assert_eq!(val["effort"], "medium");
    }

    #[test]
    fn param_low_json() {
        let mut state = EffortState::new();
        state.set_level(EffortLevel::Low);
        let val = state.effort_param().expect("should be Some for low");
        assert_eq!(val["effort"], "low");
    }

    // -- beta_header ----------------------------------------------------------

    #[test]
    fn beta_none_for_high() {
        let mut state = EffortState::new();
        state.set_level(EffortLevel::High);
        assert!(state.beta_header().is_none());
    }

    #[test]
    fn beta_present_for_medium() {
        let mut state = EffortState::new();
        state.set_level(EffortLevel::Medium);
        assert_eq!(state.beta_header(), Some(EFFORT_BETA));
    }

    #[test]
    fn beta_present_for_low() {
        let mut state = EffortState::new();
        state.set_level(EffortLevel::Low);
        assert_eq!(state.beta_header(), Some(EFFORT_BETA));
    }
}
