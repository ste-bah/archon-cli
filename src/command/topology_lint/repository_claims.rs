//! A task body's claims about repository paths, checked against the
//! repository the decomposition was grounded in (Issue-55).
//!
//! Authors grounded in an empty tree wrote bodies asserting that files which
//! exist "do not exist", and a critic that only reads prose cannot catch a
//! false statement about the filesystem. This section is deterministic: it
//! extracts every sentence that unambiguously asserts a specific backticked
//! repository-relative path exists or does not exist, and checks the path
//! against the recorded repository at its recorded base commit and in the
//! checkout. A body that says a file does not exist when it does, or exists
//! when it does not, gets a blocking `Body` finding naming the exact path and
//! the observed truth, so the body-repair loop rewrites the sentence.
//!
//! # No false positives
//!
//! Only a claim whose subject is the path itself is read: the backticked
//! path, then at most a few filler words, then an existence phrase, then a
//! terminator. "`src/lib.rs` is missing the trait impl" says nothing about
//! the file's existence and is not a claim; "the feature does not exist in
//! `src/lib.rs`" puts the path after the phrase and is not a claim either.
//! Truth is read from both the base commit and the checkout, and a claim is
//! refuted only when both agree against it: a file added or deleted since
//! the base is neither certainly present nor certainly absent, so an author
//! who read the checkout is never contradicted by history it could not see.
//! A task set without a repository record predates the record and is not
//! checked.

use std::path::Path;

use anyhow::{Context, Result};
use archon_workflow::repository_record::{RepositoryTree, read_repository_record};
use archon_workflow::task_universe::{parsing::parse_task_file, task_files_under};

use crate::command::workflow_gate::{GateFinding, GateId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Claim {
    Exists,
    Absent,
}

/// One unambiguous claim: the backticked path, what the sentence says of it,
/// and the sentence itself for the finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathClaim {
    pub(crate) path: String,
    pub(crate) claim: Claim,
    pub(crate) sentence: String,
}

mod parser;
pub(crate) use parser::extract_claims;

/// The blocking findings for the task body `text` at `path`, against the
/// repository recorded under `tasks_root`. Empty when the set has no record.
/// An error means the record or the repository could not be read:
/// operational, never a pass.
pub(crate) fn inspect(
    project_root: &Path,
    tasks_root: &Path,
    task_id: &str,
    path: &Path,
    text: &str,
) -> Result<Vec<String>> {
    let Some(record) = read_repository_record(tasks_root)? else {
        return Ok(Vec::new());
    };
    let tree = RepositoryTree::load(&record).context("loading the recorded repository tree")?;
    Ok(body_findings(&tree, project_root, task_id, path, text))
}

/// Both sections for one body: every deliverable path's observation
/// (Issue-56), then every other claim the prose makes. A path the
/// observation section already reports is not reported twice — the finding
/// that names the line count is the one the author needs.
pub(crate) fn body_findings(
    tree: &RepositoryTree,
    project_root: &Path,
    task_id: &str,
    path: &Path,
    text: &str,
) -> Vec<String> {
    // An unparsable body has no deliverable lists; the preflight reports it.
    let observed = parse_task_file(path, text)
        .map(|task| {
            super::repository_observations::findings_against(
                tree,
                project_root,
                task_id,
                text,
                &task,
            )
        })
        .unwrap_or_default();
    let reported: std::collections::BTreeSet<&str> =
        observed.iter().map(|(p, _)| p.as_str()).collect();
    let claims = claim_findings(tree, project_root, task_id, text)
        .into_iter()
        .filter(|(p, _)| !reported.contains(p.as_str()))
        .map(|(_, text)| text)
        .collect::<Vec<_>>();
    observed
        .into_iter()
        .map(|(_, text)| text)
        .chain(claims)
        .collect()
}

/// `project_root` is where a relative path that is not repository source (a
/// PRD, a task file, a project artifact) may legitimately live: an "exists"
/// claim about a path present there is about the project, not the
/// repository, and is never refuted.
#[cfg(test)]
pub(crate) fn findings_against(
    tree: &RepositoryTree,
    project_root: &Path,
    task_id: &str,
    text: &str,
) -> Vec<String> {
    claim_findings(tree, project_root, task_id, text)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// Each refuted claim with the repository-relative path it is about.
fn claim_findings(
    tree: &RepositoryTree,
    project_root: &Path,
    task_id: &str,
    text: &str,
) -> Vec<(String, String)> {
    let mut findings = Vec::new();
    for claim in extract_claims(text) {
        let Some(relative) = tree.relative_to_root(&claim.path) else {
            continue;
        };
        let truth = tree.truth(&relative);
        let in_project = !claim.path.starts_with('/') && project_root.join(&relative).exists();
        let observed = match claim.claim {
            Claim::Absent if truth.certainly_exists() => format!(
                "it exists in repository {} at base commit {} and in the checkout",
                tree.root().display(),
                tree.base_commit()
            ),
            Claim::Exists if truth.certainly_absent() && !in_project => format!(
                "it is absent from repository {} at base commit {} and from the checkout",
                tree.root().display(),
                tree.base_commit()
            ),
            _ => continue,
        };
        let said = match claim.claim {
            Claim::Absent => "does not exist",
            Claim::Exists => "exists",
        };
        let finding = format!(
            "{task_id}: the body says `{relative}` {said} (\"{}\") but {observed}; read the path under the repository root and rewrite the claim to what is there",
            excerpt(&claim.sentence)
        );
        findings.push((relative, finding));
    }
    findings
}

pub(super) fn excerpt(sentence: &str) -> String {
    const MAX: usize = 160;
    if sentence.chars().count() <= MAX {
        return sentence.to_string();
    }
    let mut cut: String = sentence.chars().take(MAX).collect();
    cut.push('…');
    cut
}

/// The set gate's findings: every task body under `root`, each as a `Body`
/// finding on the task it names.
pub(crate) fn set_findings(project_root: &Path, root: &Path) -> Result<Vec<GateFinding>> {
    let Some(record) = read_repository_record(root)? else {
        return Ok(Vec::new());
    };
    let tree = RepositoryTree::load(&record).context("loading the recorded repository tree")?;
    let mut findings = Vec::new();
    // Unreadable or unparsable task files are the shared preflight's to report.
    for path in task_files_under(root).unwrap_or_default() {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(task) = parse_task_file(&path, &raw) else {
            continue;
        };
        findings.extend(
            body_findings(&tree, project_root, &task.canonical_task_id, &path, &raw)
                .into_iter()
                .map(|text| {
                    GateFinding::new(
                        GateId::WorkflowLintTaskSet,
                        text,
                        &task.canonical_task_id,
                        Some(path.clone()),
                        archon_workflow::RemediationScope::Body,
                    )
                }),
        );
    }
    Ok(findings)
}

#[cfg(test)]
#[path = "repository_claims_tests.rs"]
mod tests;
