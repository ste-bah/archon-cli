//! Path refusals shared by acceptance authors and rooted publication policy.
use crate::task_set_contract::AcceptanceCriterion;
use std::path::Path;
#[path = "task_set_contract_policy_paths_commands.rs"]
mod commands;

pub fn acceptance_site_path_defect(
    candidate: &AcceptanceCriterion,
    project: &Path,
    repository: &Path,
    build_cache: bool,
) -> Option<String> {
    if let crate::task_set_contract::AcceptanceCheck::Command { command, cwd } = &candidate.check {
        // This is a lexical safety scan, not shell evaluation. It covers
        // path-like words, interpreter operands/program text, and the common
        // `cd .. && cd child` form; dynamic path construction remains outside
        // what a static scan can establish.
        let root_name = match cwd {
            crate::task_set_contract::TrustedCwd::ProjectRoot => "project root",
            crate::task_set_contract::TrustedCwd::RepoRoot => "repository root",
        };
        let root = match cwd {
            crate::task_set_contract::TrustedCwd::ProjectRoot => project,
            crate::task_set_contract::TrustedCwd::RepoRoot => repository,
        };
        let named_paths = commands::parent_paths(command, root);
        let scoped_command = commands::path_command(&named_paths);
        for named in named_paths {
            if let Err(refusal) = crate::acceptance_scratch::site_path_alone(
                project,
                repository,
                &scoped_command,
                *cwd,
                &named,
                build_cache,
            ) {
                let refused_path = refusal
                    .reason
                    .as_deref()
                    .and_then(|reason| {
                        reason.strip_prefix(crate::acceptance_scratch::faults::SIBLING_ROOT_REFUSED)
                    })
                    .and_then(|reason| reason.strip_prefix(": "))
                    .and_then(|reason| reason.split_once(" resolves to ").map(|(path, _)| path))
                    .map(str::to_owned)
                    .unwrap_or_else(|| archon_shell::paths::stored_text(&named));
                let reason = refusal.reason.unwrap_or_else(|| {
                    format!(
                        "alone in its site, `{}` from {root_name} resolves to `{}`, which the site does not create",
                        archon_shell::paths::stored_text(&named),
                        archon_shell::paths::stored_text(&refusal.site_target),
                    )
                });
                return Some(format!(
                    "it names `{}` from the {root_name}; {reason} (site target `{}`).",
                    refused_path,
                    archon_shell::paths::stored_text(&refusal.site_target),
                ));
            }
        }
    }
    None
}
