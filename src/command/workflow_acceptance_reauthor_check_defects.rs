//! Candidate checks refused before they reach the judge or probe.

use super::*;

/// A candidate with a host-verifiable defect never reaches the judge.
pub(super) fn check_defects(
    working: &AcceptanceContract,
    candidate: AcceptanceCriterion,
    scope: &AuthorScope,
) -> std::result::Result<AcceptanceCriterion, String> {
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, cwd } =
        &candidate.check
    {
        // This is a lexical safety scan, not shell evaluation. It covers
        // path-like words, interpreter operands/program text, and the common
        // `cd .. && cd child` form; dynamic path construction remains outside
        // what a static scan can establish.
        let root_name = match cwd {
            archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => "project root",
            archon_workflow::task_set_contract::TrustedCwd::RepoRoot => "repository root",
        };
        for named in archon_workflow::acceptance_sibling_paths::scan(command)
            .into_iter()
            .filter(|named| {
                named
                    .path
                    .components()
                    .any(|part| part == std::path::Component::ParentDir)
            })
        {
            if let Err(refusal) = archon_workflow::acceptance_scratch::site_path_alone(
                &scope.project_root,
                &scope.repository_root,
                command,
                *cwd,
                &named.path,
            ) {
                let refused_path = refusal
                    .reason
                    .as_deref()
                    .and_then(|reason| {
                        reason.strip_prefix(
                            archon_workflow::acceptance_scratch::faults::SIBLING_ROOT_REFUSED,
                        )
                    })
                    .and_then(|reason| reason.strip_prefix(": "))
                    .and_then(|reason| reason.split_once(" resolves to ").map(|(path, _)| path))
                    .map(str::to_owned)
                    .unwrap_or_else(|| named.path.display().to_string());
                let reason = refusal.reason.unwrap_or_else(|| {
                    format!(
                        "alone in its site, `{}` from {root_name} resolves to `{}`, which the site does not create (it creates `{}` and `{}` plus the siblings this check names)",
                        named.path.display(),
                        refusal.site_target.display(),
                        archon_workflow::acceptance_scratch::SITE_REPOSITORY_DIRECTORY,
                        archon_workflow::acceptance_scratch::SITE_PROJECT_DIRECTORY,
                    )
                });
                return Err(format!(
                    "check '{}': it names `{}` from the {root_name}; {reason} (site target `{}`).",
                    candidate.id,
                    refused_path,
                    refusal.site_target.display(),
                ));
            }
        }
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
