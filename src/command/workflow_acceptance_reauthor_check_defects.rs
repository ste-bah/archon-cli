//! Candidate checks refused before they reach the judge or probe.

use super::*;

/// A candidate with a host-verifiable defect never reaches the judge.
pub(super) fn check_defects(
    working: &AcceptanceContract,
    candidate: AcceptanceCriterion,
    scope: &AuthorScope,
) -> std::result::Result<AcceptanceCriterion, String> {
    if let Some(defect) = archon_workflow::task_set_contract::acceptance_site_path_defect(
        &candidate,
        &scope.project_root,
        &scope.repository_root,
        scope.build_cache,
    ) {
        return Err(format!("check '{}': {defect}", candidate.id));
    }
    let mut probe = working.clone();
    *super::entry_mut(&mut probe, &candidate.id).expect("named id") = candidate.clone();
    let prefix = format!("{}.check", candidate.id);
    let defects = super::acceptance_policy_findings(&probe)
        .into_iter()
        .filter(|finding| finding.field == prefix)
        .map(|finding| finding.message)
        .collect::<Vec<_>>();
    if defects.is_empty() {
        Ok(candidate)
    } else {
        Err(defects.join("; "))
    }
}
