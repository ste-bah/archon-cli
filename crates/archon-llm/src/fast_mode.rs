use serde_json::json;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const FAST_MODE_BETA: &str = "fast-mode-2026-02-01";

// ---------------------------------------------------------------------------
// Fast mode state
// ---------------------------------------------------------------------------

/// Tracks whether fast mode is active for API requests.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FastModeState {
    enabled: bool,
}

impl FastModeState {
    /// Create a new `FastModeState` with fast mode disabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new `FastModeState` with the given initial state.
    pub fn new_with(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Toggle fast mode on/off and return the new state.
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }

    /// Whether fast mode is currently active.
    pub fn is_active(&self) -> bool {
        self.enabled
    }

    /// Build the JSON speed parameter when fast mode is enabled.
    ///
    /// Returns `{"speed": "fast"}` when enabled, `None` otherwise.
    pub fn speed_param(&self) -> Option<serde_json::Value> {
        if self.enabled {
            Some(json!({ "speed": "fast" }))
        } else {
            None
        }
    }

    /// Return the beta header string required for fast mode.
    ///
    /// Returns `Some("fast-mode-2026-02-01")` when enabled, `None` otherwise.
    pub fn beta_header(&self) -> Option<&'static str> {
        if self.enabled {
            Some(FAST_MODE_BETA)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_disabled() {
        let state = FastModeState::new();
        assert!(!state.is_active());
    }

    #[test]
    fn toggle_enables_then_disables() {
        let mut state = FastModeState::new();
        assert!(state.toggle());
        assert!(state.is_active());
        assert!(!state.toggle());
        assert!(!state.is_active());
    }

    #[test]
    fn speed_param_when_enabled() {
        let mut state = FastModeState::new();
        state.toggle();
        let param = state.speed_param().expect("should be Some when enabled");
        assert_eq!(param["speed"], "fast");
    }

    #[test]
    fn speed_param_none_when_disabled() {
        let state = FastModeState::new();
        assert!(state.speed_param().is_none());
    }

    #[test]
    fn beta_header_when_enabled() {
        let mut state = FastModeState::new();
        state.toggle();
        assert_eq!(state.beta_header(), Some(FAST_MODE_BETA));
    }

    #[test]
    fn beta_header_none_when_disabled() {
        let state = FastModeState::new();
        assert!(state.beta_header().is_none());
    }

    #[test]
    fn multiple_toggles_converge() {
        let mut state = FastModeState::new();
        for _ in 0..10 {
            state.toggle();
        }
        // Even number of toggles -> back to disabled
        assert!(!state.is_active());
    }
}
