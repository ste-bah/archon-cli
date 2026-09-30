//! The deadline credit a session earns by waiting on a cargo target lock,
//! bounded (Issue-213 C2b).
//!
//! The wait is credited so an agent is not timed out for another agent's
//! build, but the credit is capped by `archon_tools::capped_cargo_wait_credit`:
//! at most `MAX_TIMEOUT_EXEMPT_CARGO_WAIT`, and never more than the session's
//! own budget, summed over every round of the session.

use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub(super) struct CargoCredit {
    granted: Duration,
    budget: Duration,
}

impl CargoCredit {
    /// `budget` is the session's wall-clock budget; `None` (unlimited) has no
    /// deadline to extend, so nothing is ever credited.
    pub(super) fn new(budget_secs: Option<u64>) -> Self {
        Self {
            granted: Duration::ZERO,
            budget: budget_secs.map(Duration::from_secs).unwrap_or_default(),
        }
    }

    /// The credit the wait IN PROGRESS earns now, on top of what was banked.
    pub(super) fn live(&self, session_id: &str) -> Duration {
        archon_tools::capped_cargo_wait_credit(
            self.granted,
            archon_tools::current_timeout_exempt_cargo_wait(session_id),
            self.budget,
        )
    }

    /// Take the round's finished wait off the registry and bank what the cap
    /// still allows; the returned credit is what the deadline moves by.
    pub(super) fn bank(&mut self, session_id: &str) -> Duration {
        let credit = archon_tools::capped_cargo_wait_credit(
            self.granted,
            archon_tools::take_timeout_exempt_cargo_wait(session_id),
            self.budget,
        );
        self.granted += credit;
        credit
    }

    #[cfg(test)]
    pub(super) fn granted(&self) -> Duration {
        self.granted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unlimited_session_is_never_credited() {
        let mut credit = CargoCredit::new(None);
        assert_eq!(credit.live("no-such-session"), Duration::ZERO);
        assert_eq!(credit.bank("no-such-session"), Duration::ZERO);
    }

    #[test]
    fn banked_credit_never_exceeds_the_session_budget() {
        let mut credit = CargoCredit::new(Some(60));
        credit.granted = Duration::from_secs(60);
        assert_eq!(credit.live("no-such-session"), Duration::ZERO);
        assert_eq!(credit.bank("no-such-session"), Duration::ZERO);
        assert_eq!(credit.granted(), Duration::from_secs(60));
    }
}
