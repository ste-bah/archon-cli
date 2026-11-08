//! Candidate checks refused before they reach the judge or probe.

use super::*;

/// A candidate with a host-verifiable defect never reaches the judge.
pub(super) fn check_defects(
    working: &AcceptanceContract,
    candidate: AcceptanceCriterion,
    scope: &AuthorScope,
) -> std::result::Result<AcceptanceCriterion, String> {
    let mut defects = Vec::new();
    if let Some(defect) = archon_workflow::task_set_contract::acceptance_site_path_defect(
        &candidate,
        &scope.project_root,
        &scope.repository_root,
        scope.build_cache,
    ) {
        defects.push(format!("check '{}': {defect}", candidate.id));
    }
    let mut probe = working.clone();
    *super::entry_mut(&mut probe, &candidate.id).expect("named id") = candidate.clone();
    if super::super::preflight::parse_checks(&mut probe)
        .iter()
        .any(|defect| defect.identity.subject == candidate.id)
    {
        defects.push(
            super::entry(&probe, &candidate.id)
                .expect("named id")
                .judgment
                .reason
                .clone(),
        );
    }
    let prefix = format!("{}.check", candidate.id);
    let policy = crate::command::acceptance_scratch_policy::authoring_policy(
        &scope.project_root,
        &scope.task_root,
    );
    let bound = archon_workflow::acceptance_check_environment::policy_bound_names(&policy);
    defects.extend(
        archon_workflow::task_set_contract::acceptance_policy_findings_with_bindings(
            &probe, None, &bound,
        )
        .into_iter()
        .filter(|finding| finding.field == prefix)
        .map(|finding| finding.message),
    );
    if defects.is_empty() {
        Ok(candidate)
    } else {
        Err(defects.join("; "))
    }
}

/// Keep every applicable gate reason, including a crash already seeded for
/// the same entry. The caller's finding is the repair task, verbatim.
pub(crate) fn seed_findings(
    seeds: &mut BTreeMap<String, String>,
    named: &BTreeSet<String>,
    findings: &[crate::command::workflow_gate::GateFinding],
) {
    for finding in findings
        .iter()
        .filter(|finding| named.contains(&finding.subject))
    {
        seeds
            .entry(finding.subject.clone())
            .and_modify(|text| {
                text.push('\n');
                text.push_str(&finding.text);
            })
            .or_insert_with(|| finding.text.clone());
    }
}

#[cfg(test)]
#[path = "workflow_acceptance_reauthor_text_p4_tests.rs"]
mod p4_tests;
