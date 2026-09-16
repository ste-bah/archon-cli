//! Review attribution and remediation budget.
//!
//! Which task a finding belongs to is decided by the host now
//! (`v2::review_findings`), and tested there; the prelude only reads the
//! host's attachment. What remains here is how many remediation attempts a
//! task may buy, and how a fan-out's outcomes are read.

#[cfg(test)]
#[path = "v3_prelude_remediation_budget_tests.rs"]
mod remediation_budget_tests;
#[cfg(test)]
#[path = "v3_prelude_outcomes_tests.rs"]
mod outcomes_tests;
#[cfg(test)]
#[path = "v3_prelude_roster_tests.rs"]
mod roster_tests;
