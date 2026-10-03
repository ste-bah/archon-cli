//! What a caller may steer inside one observation (Issue 255): how long
//! each check may run, and where each finished check's result goes while
//! the observation is still running.
use super::process::CheckResult;
use std::sync::Arc;

/// The operational error of a check the observation did not start, or
/// stopped only because the caller's time budget ran out: no verdict, and
/// nothing about the check itself.
pub const CHECK_DEFERRED: &str =
    "deferred: the caller's time budget ran out before this check could run to completion";

/// Asked before each check starts.
pub type AllowanceHook = Arc<dyn Fn() -> CheckAllowance + Send + Sync>;
/// Told each finished check's result.
pub type CheckHook = Arc<dyn Fn(&CheckResult) + Send + Sync>;

/// What the observation may spend on its next check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckAllowance {
    /// Run it for at most `timeout_secs` (never above the policy's own).
    /// `cut` says the bound is below the caller's per-check cap only
    /// because its budget is nearly spent: a timeout under it is then
    /// [`CHECK_DEFERRED`], never the check's own timeout.
    Run { timeout_secs: u64, cut: bool },
    /// Do not start it, nor any check after it.
    Defer,
}

/// A caller's hooks into one observation. The default changes nothing:
/// every check runs under the policy's timeout and nobody is told early.
#[derive(Clone, Default)]
pub struct ObserveHooks {
    /// Asked before each check starts.
    pub allowance: Option<AllowanceHook>,
    /// Told each check's result, redacted exactly as the observation's own
    /// record, as soon as the check and its integrity audit finish. A
    /// result told here is still voided by a later failure of the whole
    /// observation (its teardown or its live-root audit); the caller must
    /// then discard it.
    pub on_check: Option<CheckHook>,
}

impl ObserveHooks {
    /// The next check's timeout, and whether a timeout under it is a
    /// deferral; `None` when it must not start.
    pub(super) fn bound(&self, policy_timeout: u64) -> Option<(u64, bool)> {
        match self.allowance.as_ref().map(|allowance| allowance()) {
            None => Some((policy_timeout, false)),
            Some(CheckAllowance::Defer) => None,
            Some(CheckAllowance::Run { timeout_secs, cut }) => {
                let timeout = timeout_secs.clamp(1, policy_timeout);
                Some((timeout, cut && timeout < policy_timeout))
            }
        }
    }
}
