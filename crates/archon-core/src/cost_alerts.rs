use crate::config::CostConfig;

/// Action returned by [`CostAlertState::check_cost`].
#[derive(Debug, Clone, PartialEq)]
pub enum CostAlertAction {
    /// No alert needed.
    None,
    /// Session cost exceeded the warning threshold (fired once).
    Warn(String),
    /// Session cost exceeded the hard limit — caller should pause.
    HardLimitPause(String),
}

/// Tracks alert state across a session so warnings fire exactly once and hard
/// limits can be escalated by the user.
#[derive(Debug)]
pub struct CostAlertState {
    warn_fired: bool,
    hard_limit_paused: bool,
    current_limit: f64,
}

impl CostAlertState {
    /// Create a new alert state seeded from the config's hard limit.
    pub fn new(config: &CostConfig) -> Self {
        Self {
            warn_fired: false,
            hard_limit_paused: false,
            current_limit: config.hard_limit,
        }
    }

    /// Check the current session cost against thresholds and return the
    /// appropriate action.
    ///
    /// * `Warn` is returned **once** when `session_cost` first exceeds
    ///   `warn_threshold`.
    /// * `HardLimitPause` is returned when `session_cost` exceeds the active
    ///   hard limit (and the hard limit is enabled, i.e. > 0).
    /// * Otherwise `None`.
    pub fn check_cost(&mut self, session_cost: f64, config: &CostConfig) -> CostAlertAction {
        // Hard limit takes priority (checked first).
        if self.current_limit > 0.0 && session_cost >= self.current_limit {
            self.hard_limit_paused = true;
            return CostAlertAction::HardLimitPause(format!(
                "Session cost ${session_cost:.4} has reached the hard limit ${:.4}. \
                 Pipeline paused. Continue to raise the limit by 50%.",
                self.current_limit,
            ));
        }

        // Warn threshold (fire once).
        if !self.warn_fired
            && config.warn_threshold > 0.0
            && session_cost >= config.warn_threshold
        {
            self.warn_fired = true;
            return CostAlertAction::Warn(format!(
                "Session cost ${session_cost:.4} has exceeded the warning threshold ${:.4}.",
                config.warn_threshold,
            ));
        }

        CostAlertAction::None
    }

    /// Call when the user chooses to continue after a hard-limit pause.
    /// Raises the active limit by 50%.
    pub fn raise_limit(&mut self) {
        if self.current_limit > 0.0 {
            self.current_limit *= 1.5;
            self.hard_limit_paused = false;
        }
    }

    /// Reset alert state — call when thresholds are changed via ConfigTool.
    pub fn reset(&mut self, config: &CostConfig) {
        self.warn_fired = false;
        self.hard_limit_paused = false;
        self.current_limit = config.hard_limit;
    }

    /// Whether the session is currently paused at the hard limit.
    pub fn is_paused(&self) -> bool {
        self.hard_limit_paused
    }

    /// The currently active hard limit (may have been raised).
    pub fn current_limit(&self) -> f64 {
        self.current_limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(warn: f64, hard: f64) -> CostConfig {
        CostConfig {
            warn_threshold: warn,
            hard_limit: hard,
        }
    }

    #[test]
    fn no_alert_below_thresholds() {
        let config = cfg(5.0, 10.0);
        let mut state = CostAlertState::new(&config);
        assert_eq!(state.check_cost(2.0, &config), CostAlertAction::None);
    }

    #[test]
    fn warn_fires_once() {
        let config = cfg(5.0, 10.0);
        let mut state = CostAlertState::new(&config);

        let action = state.check_cost(5.5, &config);
        assert!(matches!(action, CostAlertAction::Warn(_)));

        // Second check at same cost should not warn again.
        let action2 = state.check_cost(5.5, &config);
        assert_eq!(action2, CostAlertAction::None);
    }

    #[test]
    fn hard_limit_pauses() {
        let config = cfg(5.0, 10.0);
        let mut state = CostAlertState::new(&config);

        let action = state.check_cost(10.0, &config);
        assert!(matches!(action, CostAlertAction::HardLimitPause(_)));
        assert!(state.is_paused());
    }

    #[test]
    fn raise_limit_increases_by_50_percent() {
        let config = cfg(5.0, 10.0);
        let mut state = CostAlertState::new(&config);

        // First, trigger the warn so it's out of the way.
        let action = state.check_cost(6.0, &config);
        assert!(matches!(action, CostAlertAction::Warn(_)));

        // Hit hard limit.
        let _ = state.check_cost(10.0, &config);
        assert!(state.is_paused());

        // User continues.
        state.raise_limit();
        assert!(!state.is_paused());
        assert!((state.current_limit() - 15.0).abs() < f64::EPSILON);

        // Cost below new limit is fine (warn already fired).
        assert_eq!(state.check_cost(12.0, &config), CostAlertAction::None);

        // Exceeding new limit pauses again.
        let action = state.check_cost(15.0, &config);
        assert!(matches!(action, CostAlertAction::HardLimitPause(_)));
    }

    #[test]
    fn hard_limit_zero_means_disabled() {
        let config = cfg(5.0, 0.0);
        let mut state = CostAlertState::new(&config);

        // Even a huge cost should not trigger hard limit.
        let action = state.check_cost(1000.0, &config);
        // It should trigger warn (first time), not hard limit.
        assert!(matches!(action, CostAlertAction::Warn(_)));

        // Subsequent checks return None (warn already fired, no hard limit).
        assert_eq!(state.check_cost(2000.0, &config), CostAlertAction::None);
    }

    #[test]
    fn reset_clears_state() {
        let config = cfg(5.0, 10.0);
        let mut state = CostAlertState::new(&config);

        // Fire warn.
        let _ = state.check_cost(6.0, &config);
        assert!(state.warn_fired);

        // Raise thresholds and reset.
        let new_config = cfg(20.0, 50.0);
        state.reset(&new_config);
        assert!(!state.warn_fired);
        assert!(!state.is_paused());
        assert!((state.current_limit() - 50.0).abs() < f64::EPSILON);

        // Warn should fire again at new threshold.
        let action = state.check_cost(25.0, &new_config);
        assert!(matches!(action, CostAlertAction::Warn(_)));
    }

    #[test]
    fn hard_limit_takes_priority_over_warn() {
        // Both thresholds at the same value — hard limit wins.
        let config = cfg(10.0, 10.0);
        let mut state = CostAlertState::new(&config);

        let action = state.check_cost(10.0, &config);
        assert!(matches!(action, CostAlertAction::HardLimitPause(_)));
    }

    #[test]
    fn warn_threshold_zero_means_disabled() {
        let config = cfg(0.0, 10.0);
        let mut state = CostAlertState::new(&config);

        // Should not warn even though cost > 0.
        let action = state.check_cost(5.0, &config);
        assert_eq!(action, CostAlertAction::None);

        // Hard limit still works.
        let action = state.check_cost(10.0, &config);
        assert!(matches!(action, CostAlertAction::HardLimitPause(_)));
    }
}
