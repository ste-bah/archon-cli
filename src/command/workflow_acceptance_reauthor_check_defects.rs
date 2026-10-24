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

fn leaves_selected_root(path: &std::path::Path) -> bool {
    use std::path::Component;
    let mut depth = 0usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir => return true,
            _ => return false,
        }
    }
    false
}

fn sibling_is_provided(
    selected: &std::path::Path,
    path: &std::path::Path,
    discovery: &archon_workflow::acceptance_sibling_paths::SiblingDiscovery,
) -> bool {
    let Ok(target) = archon_shell::paths::canonicalize(&selected.join(path)) else {
        return false;
    };
    discovery
        .siblings
        .iter()
        .any(|sibling| target.starts_with(&sibling.source))
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
        let discovery = archon_workflow::acceptance_sibling_paths::discover_with_diagnostics(
            selected,
            std::slice::from_ref(command),
        );
        let selected_canonical = archon_shell::paths::canonicalize(selected).ok();
        // This is a lexical safety scan, not shell evaluation.
        for named in archon_workflow::acceptance_sibling_paths::scan(command)
            .into_iter()
            .filter(|named| {
                named
                    .path
                    .components()
                    .any(|part| part == std::path::Component::ParentDir)
            })
        {
            let resolved = selected_canonical
                .as_ref()
                .map(|root| lexical_normalize(&root.join(&named.path)));
            let returns_to_selected_root = leaves_selected_root(&named.path)
                && selected_canonical.as_ref().is_some_and(|selected_root| {
                    resolved
                        .as_ref()
                        .is_some_and(|target| target.starts_with(selected_root))
                });
            let reason = if returns_to_selected_root && selected == &scope.repository_root {
                Some(
                    "repo_root names repository paths relative to repo_root; the site links `../<repository dir>` only when a project_root check names it, so this path is not always provided",
                )
            } else if returns_to_selected_root {
                Some(
                    "the site names its copy of the project root differently, so `..` and the project directory name do not lead back into it; name the path relative to project_root",
                )
            } else {
                None
            };
            let diagnostic = discovery
                .candidates
                .iter()
                .find(|item| item.path == named.path);
            let reason = reason.or_else(|| {
                diagnostic.and_then(|item| match item.reason {
                    archon_workflow::acceptance_sibling_paths::SiblingExclusionReason::MissingAtRoot => {
                        Some("discovery reports the path missing at the selected root")
                    }
                    archon_workflow::acceptance_sibling_paths::SiblingExclusionReason::ProjectRootOrAncestor => {
                        Some("discovery skips the project root or an ancestor")
                    }
                    archon_workflow::acceptance_sibling_paths::SiblingExclusionReason::SiblingTreeFromSelectedRoot => {
                        (!sibling_is_provided(selected, &named.path, &discovery))
                            .then_some("discovery found no copyable sibling source")
                    }
                })
            });
            if let Some(reason) = reason {
                return Err(format!(
                    "check '{}': it names `{}` from the {}; {reason}.",
                    candidate.id,
                    named.path.display(),
                    if selected == &scope.project_root {
                        "project root"
                    } else {
                        "repository root"
                    },
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
