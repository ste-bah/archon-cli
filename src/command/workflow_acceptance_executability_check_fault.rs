//! A probe run that ended without a verdict because of the check's own run
//! is the check's author's, never the host's.
//!
//! The observation names why a run gave no verdict
//! ([`archon_workflow::acceptance_scratch::faults`]). What the check's own
//! run caused -- output or files past a site limit, a process it started
//! that outlived its tree or held its pipes, a change to the tree it ran
//! in -- no host repair or retry can change, so it goes to the author at
//! once, naming what the check did. What may be the host's as well as the
//! check's (a signal the mutation wrapper saw, which may be the host's
//! memory pressure; a teardown the host could not verify) follows the strike
//! rule of `workflow_acceptance_executability_silent` (#356): the first time
//! it is unproven and resumable, and the same unchanged check failing so
//! again after a resume goes to its author. Everything else stays the
//! host's.

use archon_workflow::acceptance_scratch::faults::{CheckFault, classify};

use super::*;

/// Whose a run that gave no verdict is.
pub(super) enum Fault {
    Check {
        trigger: &'static str,
        remedy: &'static str,
        error: String,
    },
    Ambiguous(String),
    Host,
}

/// Whose the operational error of `result` is, by the typed cause the
/// observation recorded on it when it has one; no error (no result at all)
/// is the host's.
pub(super) fn of_result(result: Option<&CheckResult>) -> Fault {
    let Some((result, error)) =
        result.and_then(|result| Some((result, result.operational_error.as_deref()?)))
    else {
        return Fault::Host;
    };
    match classify(result.fault.as_ref(), error) {
        CheckFault::Check { trigger, remedy } => Fault::Check {
            trigger,
            remedy,
            error: error.to_string(),
        },
        CheckFault::Ambiguous => Fault::Ambiguous(error.to_string()),
        CheckFault::Host => Fault::Host,
    }
}

/// Whose a mutation run's failure to give a verdict is: its operational
/// error, else a signal its wrapper saw. The live-root guard is the host's:
/// the host chose where the check ran.
pub(super) fn of_mutation(result: &CheckResult, markers: &mutation::Markers) -> Fault {
    if result.operational_error.is_some() {
        return of_result(Some(result));
    }
    if markers.guarded(result) {
        return Fault::Host;
    }
    match markers.observed_signal(result) {
        Some(signal) => Fault::Ambiguous(format!("the check wrapper observed signal {signal}")),
        None => Fault::Host,
    }
}

/// Settle check `id` of `contract`, whose run `at` (where it ran, for the
/// finding) on `commit` gave no verdict for `fault`: the author's finding,
/// or `None` with `id` recorded unproven for `unproven`. An ambiguous fault
/// with no commit to key its strike stays the host's.
pub(super) fn settle(
    probe: &HostProbe,
    (commit, contract, id): (Option<&str>, &AcceptanceContract, &str),
    fault: Fault,
    at: &str,
    unproven: String,
) -> Option<String> {
    match (fault, commit) {
        (
            Fault::Check {
                trigger,
                remedy,
                error,
            },
            _,
        ) => Some(format!(
            "check '{id}': its own run {at} gave no verdict because {trigger} ({error}); the host cannot repair or retry that, so the check must change: {remedy}"
        )),
        (Fault::Ambiguous(why), Some(commit)) => {
            let short: String = commit.chars().take(12).collect();
            silent::strike_or_unproven(
                probe,
                commit,
                contract,
                id,
                None,
                (&why, UnprovenCause::Host),
                || {
                    (
                        format!(
                            "check '{id}': its own run {at} gave no verdict because {why}, and it did so again on the same commit {short} after a resume, so the host is not at fault; make it end by its own assertion without being stopped"
                        ),
                        format!(
                            "{unproven}; it is probed again on retry, and goes to its author if it fails so again"
                        ),
                    )
                },
            )
        }
        (Fault::Ambiguous(_) | Fault::Host, _) => {
            probe.unproven(id, unproven);
            None
        }
    }
}

/// The checks of `refs` carrying a host policy defect, each recorded
/// unproven for itself: the site refuses to resolve such a check, and run
/// beside others it would leave every one of them without a verdict. Its
/// defect is the policy layer's finding, routed there with its own scope.
pub(super) fn policy_refused(
    probe: &HostProbe,
    contract: &AcceptanceContract,
    refs: &[FrozenCommandRef],
) -> BTreeSet<String> {
    let policy = archon_workflow::task_set_contract::acceptance_policy_findings(contract);
    let mut refused = BTreeSet::new();
    for reference in refs {
        let prefix = format!("{}.", reference.acceptance_id);
        let defects: Vec<&str> = (policy.iter())
            .filter(|finding| finding.field.starts_with(&prefix))
            .map(|finding| finding.message.as_str())
            .collect();
        if !defects.is_empty() {
            let why = format!(
                "it carries a host policy defect, so the host never runs it: {}",
                defects.join("; ")
            );
            probe.unproven(&reference.acceptance_id, why);
            refused.insert(reference.acceptance_id.clone());
        }
    }
    refused
}

impl HostProbe {
    /// Whether `id` is recorded unproven by this probe.
    pub(super) fn is_unproven(&self, id: &str) -> bool {
        (self.unproven.lock().expect("unproven lock")).contains_key(id)
    }
}
