//! Review attribution and remediation budget.
//!
//! Split in two to hold the 500-line source ceiling, along the seam the file
//! already carried: one module decides which task a finding belongs to, the
//! other decides how many remediation attempts that task may buy. They share
//! no fixture and no helper — only the prelude they both read.

#[cfg(test)]
#[path = "v3_prelude_review_attribution_tests.rs"]
mod review_attribution_tests;

#[cfg(test)]
#[path = "v3_prelude_remediation_budget_tests.rs"]
mod remediation_budget_tests;
