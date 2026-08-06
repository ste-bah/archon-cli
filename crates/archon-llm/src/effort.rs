use serde_json::json;
use std::fmt;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Beta header required whenever `output_config.effort` is on the wire.
/// Public so the Anthropic adapter can both send it and recognise it in a
/// rejection body without duplicating the literal.
pub const EFFORT_BETA: &str = "effort-2025-11-24";

// ---------------------------------------------------------------------------
// Effort level enum
// ---------------------------------------------------------------------------

/// Controls the reasoning effort the model should apply.
///
/// The canonical ladder is `Low < Medium < High < Max`. Providers project it
/// onto whatever tiers they actually accept, clamping DOWN where a rung does
/// not exist — see `clamp_reasoning_effort` (Codex) and `effective_effort`
/// (Anthropic). Adding a rung here therefore never breaks a provider; it just
/// saturates at that provider's ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortLevel {
    Max,
    High,
    Medium,
    Low,
}

impl fmt::Display for EffortLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Max => "max",
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        };
        f.write_str(s)
    }
}

impl EffortLevel {
    /// Position on the canonical ladder, ascending: `Low` = 0, `Max` = 3.
    ///
    /// Used to compare tiers without relying on the enum's declaration order,
    /// which is descending for readability. `ultrathink` uses this to escalate
    /// without ever lowering an explicitly-set level.
    pub fn rank(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
            Self::Max => 3,
        }
    }

    /// The higher of two tiers on the canonical ladder.
    pub fn raised_to(self, other: Self) -> Self {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
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
    /// `High` and `Max`.
    ///
    /// `High` maps to `None` because omitting `output_config.effort` IS high
    /// on the Anthropic API. `Max` maps to `None` for the same reason: the
    /// Anthropic ladder stops at high, so `Max` clamps onto it here and is
    /// expressed instead through the `thinking` parameter. This helper is
    /// Anthropic-shaped — do NOT reuse it for OpenAI-compatible backends,
    /// where an absent field means "no reasoning at all" rather than "high".
    pub fn effort_param(&self) -> Option<serde_json::Value> {
        match self.level {
            EffortLevel::High | EffortLevel::Max => None,
            other => Some(json!({ "effort": other.to_string() })),
        }
    }

    /// Return the beta header string required for non-default effort levels.
    ///
    /// Returns `Some("effort-2025-11-24")` for `Medium`/`Low`, `None` for
    /// `High`/`Max`. Kept in lockstep with [`Self::effort_param`]: the beta is
    /// only required when the parameter is actually sent.
    pub fn beta_header(&self) -> Option<&'static str> {
        match self.level {
            EffortLevel::High | EffortLevel::Max => None,
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
        "max" => Ok(EffortLevel::Max),
        "high" => Ok(EffortLevel::High),
        "medium" | "med" => Ok(EffortLevel::Medium),
        "low" => Ok(EffortLevel::Low),
        _ => Err(format!(
            "invalid effort level: '{s}' (expected low, medium, high, or max)"
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

    // -- max tier (#123) ------------------------------------------------------

    #[test]
    fn display_and_parse_max() {
        assert_eq!(EffortLevel::Max.to_string(), "max");
        assert_eq!(parse_level("max"), Ok(EffortLevel::Max));
        assert_eq!(parse_level(" MAX "), Ok(EffortLevel::Max));
    }

    #[test]
    fn parse_error_lists_all_four_tiers() {
        let err = parse_level("turbo").expect_err("turbo is not a tier");
        for tier in ["low", "medium", "high", "max"] {
            assert!(err.contains(tier), "error should mention {tier}: {err}");
        }
    }

    #[test]
    fn rank_is_ascending_and_independent_of_declaration_order() {
        assert!(EffortLevel::Low.rank() < EffortLevel::Medium.rank());
        assert!(EffortLevel::Medium.rank() < EffortLevel::High.rank());
        assert!(EffortLevel::High.rank() < EffortLevel::Max.rank());
    }

    #[test]
    fn raised_to_never_lowers() {
        assert_eq!(
            EffortLevel::Low.raised_to(EffortLevel::Max),
            EffortLevel::Max
        );
        assert_eq!(
            EffortLevel::Max.raised_to(EffortLevel::Low),
            EffortLevel::Max
        );
        assert_eq!(
            EffortLevel::High.raised_to(EffortLevel::High),
            EffortLevel::High
        );
    }

    /// `Max` must behave like `High` on the Anthropic-shaped helpers: the
    /// Anthropic ladder stops at high, so the parameter is omitted and the
    /// beta is not required.
    #[test]
    fn max_omits_param_and_beta_like_high() {
        let mut state = EffortState::new();
        state.set_level(EffortLevel::Max);
        assert!(state.effort_param().is_none());
        assert!(state.beta_header().is_none());
    }
}
