//! Candidate checks refused before they reach the judge or probe.

use super::*;

fn lexical_normalize(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// A candidate with a host-verifiable defect never reaches the judge.
pub(super) fn check_defects(
    working: &AcceptanceContract,
    candidate: AcceptanceCriterion,
    scope: &AuthorScope,
) -> std::result::Result<AcceptanceCriterion, String> {
    if let archon_workflow::task_set_contract::AcceptanceCheck::Command { command, cwd } =
        &candidate.check
    {
        let selected = match cwd {
            archon_workflow::task_set_contract::TrustedCwd::ProjectRoot => &scope.project_root,
            archon_workflow::task_set_contract::TrustedCwd::RepoRoot => &scope.repository_root,
        };
        let selected_canonical = archon_shell::paths::canonicalize(selected).ok();
        for named in archon_workflow::acceptance_sibling_paths::scan(command)
            .into_iter()
            .filter(|named| {
                named.required
                    && named
                        .path
                        .components()
                        .any(|part| part == std::path::Component::ParentDir)
            })
        {
            let target = selected.join(&named.path);
            let lexical_target = selected_canonical
                .as_ref()
                .map(|root| lexical_normalize(&root.join(&named.path)));
            let canonical_target = archon_shell::paths::canonicalize(&target).ok();
            let project_root = archon_shell::paths::canonicalize(&scope.project_root).ok();
            let repository_root = archon_shell::paths::canonicalize(&scope.repository_root).ok();
            let other_root = [
                (project_root.as_ref(), "project root"),
                (repository_root.as_ref(), "repository root"),
            ]
            .into_iter()
            .find(|(root, _)| {
                root.is_some_and(|root| {
                    Some(root) != selected_canonical.as_ref()
                        && (canonical_target
                            .as_ref()
                            .is_some_and(|target| target.starts_with(root))
                            || lexical_target
                                .as_ref()
                                .is_some_and(|target| target.starts_with(root)))
                })
            });
            if let Some((_, label)) = other_root {
                return Err(format!(
                    "check '{}': it names `{}` from the {} into the {}, which is outside the selected root and is absent from the probe copy. Never reach one root from the other with `..`: the roots do not sit the same way in every copy. Choose the root that contains the path and name it relative to that root.",
                    candidate.id,
                    named.path.display(),
                    if selected == &scope.project_root {
                        "project root"
                    } else {
                        "repository root"
                    },
                    label,
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
