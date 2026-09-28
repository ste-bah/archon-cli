//! Focused-verification outcome adjudication.
//!
//! Everything here decides whether a verification branch's self-report may be
//! believed. A verifier reports its own `commands_run`, its own status and its
//! own coverage, so the host re-reads that report against evidence it can check
//! itself: did any command actually succeed, did the filters it named match any
//! tests, and — when the item declared a deliverable contract — does the
//! contract still hold when the HOST runs the verifier rather than the audited
//! branch. Each check fails closed; "we could not check" is never a pass.
//!
//! The failure side is classified rather than merely rejected, because the
//! lifecycle has to know whether to retry the verification, route the work to
//! remediation, or stop.
//!
//! It sits in this crate rather than the binary because every input and output
//! is a type this crate owns; nothing here touches CLI state, config layering,
//! or the terminal.

mod baseline_pre_existing;
pub mod baseline_rule;
pub mod baseline_run_base;
mod contracts;
mod failure_class;
mod normalize;
pub mod path_ownership;
pub mod regression_gate;
mod signals;
pub mod unowned_paths;

pub use crate::v2::write::test_baseline_verification::{
    VerificationBaselineContext, establish_verification_baseline,
};
pub use baseline_rule::{
    baseline_by_item, enforce_baseline_tests, enforce_baseline_tests_excusing,
    stamp_baseline_tests_input, stamp_baseline_tests_input_at,
};
pub use baseline_run_base::{
    BaseRedTest, ExcusedRedTests, RunBaseRedContext, UNOWNED_RED_GAP_ID, excuse_run_base_red_tests,
};
pub use contracts::{
    enforce_declared_contracts, enforce_declared_contracts_watched, mark_branch_operational,
};
pub(crate) use normalize::is_evidenced_pre_existing_failure;
pub use normalize::{normalize_focused_verification_outcome, stamp_focused_verification_input};
pub use path_ownership::{
    PATH_OWNERSHIP_INPUT_KEY, PathOwnership, path_ownership_for, stamp_path_ownership_from_universe,
};
pub use unowned_paths::{
    BranchScope, FLAGGED_SEVERITY_MARKER, UNOWNED_PATH_GAP_PREFIX, flag_unowned_path_gaps,
    gap_is_unowned_path, scope_by_item,
};
