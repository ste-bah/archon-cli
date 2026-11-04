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
//! host's. Issue 554: a mutation run that gave no verdict for a reason the
//! host cannot tell apart from the check's own run (no exit status, an input
//! with no recorded state, an input left unrestored) follows that strike
//! rule too, and one whose own text the shell rejects in parse-only mode
//! is its author's.

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
    Availability(crate::command::workflow_host_fault::Class),
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
    if let Some(archon_workflow::acceptance_scratch::faults::FaultCause::SiteSetupIo {
        raw_os_error,
    }) = result.fault
    {
        return Fault::Availability(crate::command::workflow_host_fault::io_class(
            &std::io::Error::from_raw_os_error(raw_os_error),
        ));
    }
    match classify(result.fault.as_ref(), error) {
        CheckFault::Check { trigger, remedy } => Fault::Check {
            trigger,
            remedy,
            error: error.to_string(),
        },
        CheckFault::Ambiguous => Fault::Ambiguous(error.to_string()),
        CheckFault::Host
            if matches!(
                error,
                archon_workflow::acceptance_scratch::CHECK_TIMED_OUT
                    | archon_workflow::acceptance_scratch::CHECK_DEFERRED
            ) =>
        {
            Fault::Availability(crate::command::workflow_host_fault::Class::Transient)
        }
        CheckFault::Host => Fault::Host,
    }
}

/// Whose a mutation run's failure to give a verdict is: its operational
/// error, else a signal its wrapper saw. The live-root guard is the host's:
/// the host chose where the check ran.
///
/// Issue 554: a run that recorded no exit status whose own text `command`
/// the shell that runs it rejects in parse-only mode is its author's at
/// once, with the shell's error. Any other run without a verdict (`reason`: no exit status,
/// an input with no recorded state, an input left unrestored) may be the
/// host's as well as the check's, so it follows the strike rule: the same
/// unchanged check failing so again goes to its author.
pub(super) fn of_mutation(
    result: &CheckResult,
    markers: &mutation::Markers,
    command: &str,
    reason: &str,
) -> Fault {
    if result.operational_error.is_some() {
        return of_result(Some(result));
    }
    if markers.guarded(result) {
        return Fault::Host;
    }
    if reason.contains("parent snapshot failed") {
        return Fault::Availability(crate::command::workflow_host_fault::Class::Permanent);
    }
    if let Some(signal) = markers.observed_signal(result) {
        return Fault::Ambiguous(format!("the check wrapper observed signal {signal}"));
    }
    // Decided by the check's own text alone: a run's stderr cannot say which
    // process printed a syntax error.
    if markers.status(result).is_none()
        && let Some(error) = archon_workflow::acceptance_check_shell::shell_parse_error(command)
    {
        return Fault::Check {
            trigger: "the shell that runs it rejects its text in parse-only mode, so the check never recorded its exit status",
            remedy: "fix exactly that shell syntax so the whole check parses (`sh -n` accepts it), and keep every assertion and every call into the deliverable",
            error,
        };
    }
    Fault::Ambiguous(reason.to_string())
}

#[derive(Clone, Copy)]
pub(super) enum Phase {
    Baseline,
    Mutation,
}

impl Phase {
    fn strike_key(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Mutation => "fault",
        }
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
    settle_in_phase(
        probe,
        (commit, contract, id),
        fault,
        (at, Phase::Baseline),
        unproven,
    )
}

pub(super) fn settle_in_phase(
    probe: &HostProbe,
    (commit, contract, id): (Option<&str>, &AcceptanceContract, &str),
    fault: Fault,
    (at, phase): (&str, Phase),
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
            let remedy = (!why.starts_with("signal ")
                && !why.contains("observed signal")
                && !why.contains("did not record its exit status"))
            .then(|| input_state_remedy(&why))
            .flatten()
            .unwrap_or_else(|| "make it end by its own assertion without being stopped".into());
            silent::strike_mutation(
                probe,
                commit,
                contract,
                id,
                phase.strike_key(),
                (&why, UnprovenCause::Host),
                || {
                    (
                        format!(
                            "check '{id}': its own run {at} gave no verdict because {why}, and it did so again on the same commit {short} after a resume, so the host is not at fault; {remedy}"
                        ),
                        format!(
                            "{unproven}; it is probed again on retry, and goes to its author if it fails so again"
                        ),
                    )
                },
            )
        }
        (Fault::Availability(class), _) => {
            probe.unproven_as(id, unproven, UnprovenCause::from_class(class));
            None
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

fn input_state_remedy(why: &str) -> Option<String> {
    let states = why.split_once("input states: ")?.1.split(';').next()?;
    let clauses: Vec<_> = states.split(", ").filter_map(|entry| {
        let (path, state) = entry.split_once('=')?;
        (state != "restored" && state != "original-recreated" && state != "unreported").then(|| format!("it left `{path}` in state {state}; do not create, replace, link or remove {path} while it runs"))
    }).collect();
    (!clauses.is_empty()).then(|| clauses.join("; "))
}
