use super::*;

pub(super) fn frozen_check_plan(
    contract: &AcceptanceContract,
) -> Vec<(String, bool, Option<String>)> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| {
            let accepted = entry.judgment.verdict == JudgeDecision::Accepted;
            let reason = (!accepted).then(|| {
                format!(
                    "freeze does not run this entry because its frozen judgment is {:?}; only Accepted entries are executable",
                    entry.judgment.verdict
                )
            });
            (entry.id.clone(), accepted, reason)
        })
        .collect()
}

pub(super) fn not_run_checks(plan: &[(String, bool, Option<String>)]) -> Vec<CheckResult> {
    plan.iter()
        .filter(|(_, accepted, _)| !accepted)
        .map(|(id, _, reason)| CheckResult {
            check_id: id.clone(),
            verdict: "not_run".into(),
            really_ran: false,
            setup_failure: false,
            real_failure: false,
            sibling_paths: Vec::new(),
            findings: Vec::new(),
            not_run_reason: reason.clone(),
            regression_guard: None,
            remediation_route: None,
        })
        .collect()
}

/// The progress line's outcome for a regression guard: its basis, which the
/// probe redacted with every value bound into the check's environment.
pub(super) fn guard_outcome(basis: &str) -> String {
    format!("passed; regression guard: {basis}")
}

/// Published checks the site path rule refuses: each goes to its author.
pub(super) fn site_refused_checks(refusals: &BTreeMap<String, String>) -> Vec<CheckResult> {
    refusals
        .iter()
        .map(|(id, reason)| CheckResult {
            check_id: id.clone(),
            verdict: "needs_remediation".into(),
            really_ran: false,
            setup_failure: true,
            real_failure: false,
            sibling_paths: Vec::new(),
            findings: vec![format!(
                "check '{id}' was not run: its published text breaks the site path rule that the first author and every publication apply: {reason}"
            )],
            not_run_reason: None,
            regression_guard: None,
            remediation_route: Some(RemediationRoute::Check),
        })
        .collect()
}
