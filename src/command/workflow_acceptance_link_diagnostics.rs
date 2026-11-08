//! Observations only: these values never enter publication state.
use super::executability::mutation::{ExclusionReason, input_discovery_for_check};
use super::*;
use archon_workflow::repository_record::{RepositoryRecordV1, RepositoryTree};
use archon_workflow::task_set_contract::{AcceptanceCheck, TrustedCwd};
use std::collections::BTreeMap;

#[derive(Debug, PartialEq, Eq)]
struct Link {
    task: String,
    check: String,
    membership: bool,
    reads_deliverable: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Diagnostics {
    links: Vec<Link>,
    membership_only: Vec<String>,
    unowned: Vec<(String, PathBuf)>,
    base_unavailable: Option<String>,
    outside: Vec<(String, PathBuf)>,
    unconfirmed: Vec<(String, PathBuf)>,
}

fn absolute(root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    let joined = if path.has_root() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let mut result = PathBuf::new();
    for part in joined.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Pure comparison of discovered reads and the selected immutable tree.
/// Directory operands link and own artifacts inside them; proof decides relevance.
fn evaluate(
    skeleton: &TaskSkeleton,
    contract: &AcceptanceContract,
    repository: &Path,
    reads: &BTreeMap<String, BTreeSet<PathBuf>>,
    base: Option<&RepositoryTree>,
) -> Diagnostics {
    let mut result = Diagnostics {
        base_unavailable: base
            .is_none()
            .then(|| "no base commit: checkout has no usable commit".into()),
        ..Diagnostics::default()
    };
    let owners: Vec<Vec<PathBuf>> = skeleton
        .tasks
        .iter()
        .map(|task| {
            task.deliverable_contracts
                .iter()
                .map(|d| absolute(repository, &d.artifact_path))
                .collect()
        })
        .collect();
    for (task, owned) in skeleton.tasks.iter().zip(&owners) {
        let start = result.links.len();
        for check in contract.acceptance.iter().chain(&contract.supplementary) {
            let membership = archon_workflow::v2::acceptance_stage::coverage::answers_for(
                check,
                &task.implements,
            );
            let reads_deliverable = reads.get(&check.id).is_some_and(|paths| {
                paths.iter().any(|path| {
                    owned
                        .iter()
                        .any(|owner| path.starts_with(owner) || owner.starts_with(path))
                })
            });
            if membership || reads_deliverable {
                result.links.push(Link {
                    task: task.task_id.clone(),
                    check: check.id.clone(),
                    membership,
                    reads_deliverable,
                });
            }
        }
        let links = &result.links[start..];
        if !links.is_empty()
            && links
                .iter()
                .all(|link| link.membership && !link.reads_deliverable)
        {
            result.membership_only.push(task.task_id.clone());
        }
    }
    if let Some(tree) = base {
        for (check, paths) in reads {
            for path in paths {
                let Some(relative) = tree.relative_to_root(&path.to_string_lossy()) else {
                    result.outside.push((check.clone(), path.clone()));
                    continue;
                };
                let present = relative.is_empty() || tree.exists_at_base(&relative);
                // A directory operand is owned when an owned artifact lies inside it too.
                let owned = owners
                    .iter()
                    .flatten()
                    .any(|owner| path.starts_with(owner) || owner.starts_with(path));
                if !present && !owned {
                    result.unowned.push((check.clone(), path.clone()));
                }
            }
        }
    }
    result
}

fn discover(
    skeleton: &TaskSkeleton,
    contract: &AcceptanceContract,
    project: &Path,
    repository: Option<&Path>,
    tasks: &Path,
) -> Diagnostics {
    let project = project
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap_or_else(|_| project.to_path_buf());
    let selected = repository;
    let repository = repository
        .unwrap_or(&project)
        .canonicalize()
        .map(archon_shell::paths::plain)
        .unwrap_or_else(|_| repository.unwrap_or(&project).to_path_buf());
    let mut reads = BTreeMap::new();
    let mut unconfirmed = Vec::new();
    for check in contract.acceptance.iter().chain(&contract.supplementary) {
        let root = match check.check {
            AcceptanceCheck::Command {
                cwd: TrustedCwd::RepoRoot,
                ..
            } => &repository,
            _ => &project,
        };
        let discovery = input_discovery_for_check(check, &project, &repository, None);
        unconfirmed.extend(
            discovery
                .unconfirmed_reads
                .iter()
                .map(|path| (check.id.clone(), absolute(root, path))),
        );
        let paths = discovery
            .candidates
            .into_iter()
            .filter(|candidate| {
                !discovery.unconfirmed_reads.contains(&candidate.path)
                    && !candidate.message_mention
                    && !candidate.written_ancestor
                    && candidate.exclusion != Some(ExclusionReason::CheckWritesPathInText)
            })
            .map(|candidate| candidate.path)
            .chain(discovery.unshaped_missing)
            .chain(discovery.absolute_reads)
            .map(|path| absolute(root, &path))
            .collect();
        reads.insert(check.id.clone(), paths);
    }
    let base = selected
        .ok_or_else(|| "repository not selectable".to_string())
        .and_then(|_| {
            super::executability::Baseline::for_task_set(&repository, tasks)
                .ok_or_else(|| "no base commit: checkout has no usable commit".to_string())
        })
        .and_then(|base| {
            RepositoryTree::load(&RepositoryRecordV1 {
                schema_version:
                    archon_workflow::repository_record::REPOSITORY_RECORD_SCHEMA_VERSION,
                repository_root: base.repository.to_string_lossy().into_owned(),
                base_commit: base.commit,
                decomposition_run_id: String::new(),
                recorded_at: String::new(),
            })
            .map_err(|error| format!("base tree could not be loaded: {error}"))
        });
    let mut result = evaluate(skeleton, contract, &repository, &reads, base.as_ref().ok());
    result.base_unavailable = base.as_ref().err().cloned();
    result.unconfirmed = unconfirmed
        .into_iter()
        .filter(|(_, path)| {
            let owned = skeleton
                .tasks
                .iter()
                .flat_map(|task| &task.deliverable_contracts)
                .map(|d| absolute(&repository, &d.artifact_path))
                .any(|owner| path.starts_with(&owner) || owner.starts_with(path));
            let present = base.as_ref().ok().is_some_and(|tree| {
                tree.relative_to_root(&path.to_string_lossy())
                    .is_some_and(|relative| relative.is_empty() || tree.exists_at_base(&relative))
            });
            !owned && !present
        })
        .collect();
    result
}

/// Collect diagnostics without terminal output, including in the in-process caller.
pub(super) fn emit(
    skeleton: &TaskSkeleton,
    contract: &AcceptanceContract,
    project: &Path,
    tasks: &Path,
) -> Vec<String> {
    let repository = super::executability::diagnostic_repository(project, tasks);
    let mut diagnostics = discover(
        skeleton,
        contract,
        project,
        repository.as_deref().ok(),
        tasks,
    );
    if let Err(reason) = repository {
        diagnostics.base_unavailable = Some(format!("repository not selectable: {reason}"));
    }
    lines(&diagnostics)
}

fn lines(diagnostics: &Diagnostics) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("input-link: links={}", diagnostics.links.len()));
    for task in &diagnostics.membership_only {
        lines.push(format!("input-link: membership_only task={task}"));
    }
    if let Some(reason) = &diagnostics.base_unavailable {
        lines.push(format!(
            "input-link: base unavailable: {reason}; input ownership was not evaluated"
        ));
    }
    for (check, path) in &diagnostics.unowned {
        lines.push(format!(
            "input-link: unowned_check_input check={check} path={}",
            path.display()
        ));
    }
    for (check, path) in &diagnostics.unconfirmed {
        lines.push(format!(
            "input-link: unconfirmed_read check={check} path={}",
            path.display()
        ));
    }
    for (check, path) in &diagnostics.outside {
        lines.push(format!(
            "input-link: outside_repository check={check} path={}",
            path.display()
        ));
    }
    lines
}

pub(super) fn persist(tasks: &Path, lines: &[String]) {
    let log = tasks.join(".decompose.log");
    let previous = std::fs::read_to_string(&log).unwrap_or_default();
    let mut seen: BTreeSet<_> = previous.lines().map(str::to_owned).collect();
    for line in lines.iter().filter(|line| {
        [
            "input-link: membership_only ",
            "input-link: unowned_check_input ",
            "input-link: unconfirmed_read ",
            "input-link: base unavailable:",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
    }) {
        if seen.insert(line.clone())
            && let Err(error) =
                crate::command::workflow_decompose_log::append_nofollow_line(&log, line)
        {
            tracing::warn!(%error, "could not persist input diagnostics");
        }
    }
}

pub(super) fn child(
    skeleton: &TaskSkeleton,
    contract: &AcceptanceContract,
    project: &Path,
    tasks: &Path,
) {
    let lines = emit(skeleton, contract, project, tasks);
    persist(tasks, &lines);
    for line in lines {
        eprintln!("{line}");
    }
}

#[cfg(test)]
#[path = "workflow_acceptance_link_diagnostics_tests.rs"]
mod tests;
