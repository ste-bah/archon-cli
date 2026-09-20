//! Every repository file the PRD names by path has an owning task (Issue-55).
//!
//! A PRD that says "extend `crates/x/src/lib.rs`" has named a file; a
//! decomposition in which no task lists that file will not touch it, and the
//! run reports success without it. This section is a set comparison, like
//! coverage: the PRD's path literals that exist in the recorded repository at
//! its base commit, against the paths the tasks own. It reads no prose and
//! judges nothing — a path either has an owner or it does not.
//!
//! Ownership is generous so that the finding is never a quibble: a task owns
//! a PRD-named path when one of its owned paths is that path, lies under it
//! (the PRD named a directory), or is an ancestor of it (the task owns the
//! directory). Owned paths are the skeleton's `deliverable_contracts`
//! artifact paths at the skeleton gate, plus each body's `Files Expected to
//! Change` and shared append targets at the set gate. A task set without a
//! repository record predates the record and is not checked.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};
use archon_workflow::repository_record::{RepositoryTree, read_repository_record};
use archon_workflow::task_skeleton::TaskSkeleton;
use archon_workflow::task_universe::{parsing::parse_task_file, task_files_under};

use crate::command::workflow_gate::{GateFinding, GateId};

/// The PRD's path literals — backticked or bare — that exist in the
/// repository at the recorded base commit, repository-relative.
pub(crate) fn prd_named_repository_paths(tree: &RepositoryTree, prd: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut fenced = false;
    for line in prd.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        // Markdown punctuation is a separator: a link target `[text](path)`
        // and a parenthesised mention `(see path)` both yield the bare path.
        let separators = |c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '`' | '|'
                        | '('
                        | ')'
                        | '['
                        | ']'
                        | '{'
                        | '}'
                        | '<'
                        | '>'
                        | '"'
                        | '\''
                        | ','
                        | ';'
                )
        };
        for raw in line.split(separators) {
            // Trailing sentence punctuation and emphasis go; a leading `./`
            // stays, so the normaliser can read it as repository-relative.
            let token = raw
                .trim_start_matches(['*', '#', ':'])
                .trim_end_matches(['.', ':', '*', '#']);
            let Some(candidate) = path_like(token) else {
                continue;
            };
            let Some(relative) = tree.relative_to_root(&candidate) else {
                continue;
            };
            if !relative.is_empty() && tree.exists_at_base(&relative) {
                found.insert(relative);
            }
        }
    }
    found
}

/// A token that could name a file or directory: a `/` or a file extension,
/// no glob or template characters, not a URL. Existence in the repository is
/// what makes it a path; this only says it is worth looking up.
fn path_like(token: &str) -> Option<String> {
    if token.len() < 3
        || token.contains("://")
        || token.starts_with('~')
        || token.starts_with("..")
        || token.contains("/..")
        || token.contains(['*', '{', '}', '$', '<', '>', '|', '=', '"', '\'', '@'])
    {
        return None;
    }
    let has_slash = token.contains('/');
    let last = token.rsplit('/').next().unwrap_or(token);
    let has_extension = last.rsplit_once('.').is_some_and(|(stem, extension)| {
        !stem.is_empty()
            && !extension.is_empty()
            && extension.len() <= 8
            && extension.chars().all(|c| c.is_ascii_alphanumeric())
            && extension.chars().any(|c| c.is_ascii_alphabetic())
    });
    if !has_slash && !has_extension {
        return None;
    }
    let absolute = token.starts_with('/');
    let normalized = archon_workflow::repository_record::normalize_relative(token);
    if normalized.is_empty() {
        return None;
    }
    Some(if absolute {
        format!("/{normalized}")
    } else {
        normalized
    })
}

/// Does some owned path cover `prd_path`: equal to it, under it, or above it.
fn owned(prd_path: &str, owned_paths: &BTreeSet<String>) -> bool {
    owned_paths.iter().any(|owner| {
        owner == prd_path
            || owner.starts_with(&format!("{prd_path}/"))
            || prd_path.starts_with(&format!("{owner}/"))
    })
}

fn normalized_owned<'a>(
    tree: &RepositoryTree,
    paths: impl Iterator<Item = &'a str>,
) -> BTreeSet<String> {
    paths
        .filter_map(|path| tree.relative_to_root(path.trim()))
        .filter(|path| !path.is_empty())
        .collect()
}

/// The PRD-named repository paths no owned path covers, in order.
pub(crate) fn unowned_prd_paths<'a>(
    tree: &RepositoryTree,
    prd: &str,
    owned_paths: impl Iterator<Item = &'a str>,
) -> Vec<String> {
    let owners = normalized_owned(tree, owned_paths);
    prd_named_repository_paths(tree, prd)
        .into_iter()
        .filter(|path| !owned(path, &owners))
        .collect()
}

fn finding_text(path: &str, tree: &RepositoryTree, is_dir: bool) -> String {
    format!(
        "repository {} `{path}` is named by the PRD and exists at base commit {} but no task owns it; give it an owning task (a deliverable_contracts artifact_path in the skeleton, or Files Expected to Change in a body){}",
        if is_dir { "directory" } else { "file" },
        tree.base_commit(),
        if is_dir {
            " — a task owning any path under it counts"
        } else {
            ""
        }
    )
}

/// The skeleton gate's findings: each PRD-named repository path no task's
/// deliverable contracts own. Empty for a task set without a record.
pub(crate) fn skeleton_findings(
    tasks_root: &Path,
    prd: &str,
    skeleton: &TaskSkeleton,
) -> Result<Vec<String>> {
    let Some(record) = read_repository_record(tasks_root)? else {
        return Ok(Vec::new());
    };
    let tree = RepositoryTree::load(&record).context("loading the recorded repository tree")?;
    let owned = skeleton
        .tasks
        .iter()
        .flat_map(|task| task.deliverable_contracts.iter())
        .map(|contract| contract.artifact_path.as_str());
    Ok(unowned_prd_paths(&tree, prd, owned)
        .into_iter()
        .map(|path| {
            let is_dir = tree.is_dir_at_base(&path);
            finding_text(&path, &tree, is_dir)
        })
        .collect())
}

/// The set gate's findings: owned paths are every body's deliverable
/// contracts, `Files Expected to Change` and shared append targets, plus the
/// frozen skeleton's contracts when one exists.
pub(crate) fn set_findings(root: &Path) -> Result<Vec<GateFinding>> {
    let Some(record) = read_repository_record(root)? else {
        return Ok(Vec::new());
    };
    let tree = RepositoryTree::load(&record).context("loading the recorded repository tree")?;
    let (claims, _) = crate::command::topology_task_graph::task_requirement_claims_tolerant(root)
        .map_err(|error| {
        anyhow::anyhow!("reading task claims under {}: {error}", root.display())
    })?;
    let Some(prd_path) = super::coverage::resolve_prd(root, &claims)? else {
        return Ok(Vec::new());
    };
    let prd = std::fs::read_to_string(&prd_path)
        .with_context(|| format!("reading PRD {}", prd_path.display()))?;
    let mut owned: Vec<String> = Vec::new();
    for path in task_files_under(root).unwrap_or_default() {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(task) = parse_task_file(&path, &raw) else {
            continue;
        };
        owned.extend(
            task.deliverable_contracts
                .iter()
                .map(|c| c.artifact_path.clone()),
        );
        owned.extend(task.files_expected_to_change.iter().cloned());
        owned.extend(task.shared_append_target_files.iter().cloned());
    }
    let skeleton_path = root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
    if let Ok(bytes) = std::fs::read(&skeleton_path)
        && let Ok(skeleton) = serde_json::from_slice::<TaskSkeleton>(&bytes)
    {
        owned.extend(
            skeleton
                .tasks
                .iter()
                .flat_map(|task| task.deliverable_contracts.iter())
                .map(|contract| contract.artifact_path.clone()),
        );
    }
    let source = if skeleton_path.exists() {
        skeleton_path
    } else {
        prd_path
    };
    Ok(
        unowned_prd_paths(&tree, &prd, owned.iter().map(String::as_str))
            .into_iter()
            .map(|path| {
                let is_dir = tree.is_dir_at_base(&path);
                GateFinding::new(
                    GateId::WorkflowLintTaskSet,
                    finding_text(&path, &tree, is_dir),
                    path,
                    Some(source.clone()),
                    archon_workflow::RemediationScope::Skeleton,
                )
            })
            .collect(),
    )
}

#[cfg(test)]
#[path = "owner_coverage_tests.rs"]
mod tests;
