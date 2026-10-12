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
    pub(crate) scope: Option<String>,
    pub(crate) scoped_join: bool,
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
        .filter(|(paths, _)| !paths.iter().any(|p| reported.contains(p.as_str())))
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

/// Each refuted claim with the repository-relative paths it is about. A
/// claim with several candidate paths (a scope several directories end with)
/// is refuted only when every candidate refutes it.
fn claim_findings(
    tree: &RepositoryTree,
    project_root: &Path,
    task_id: &str,
    text: &str,
) -> Vec<(Vec<String>, String)> {
    let mut findings = Vec::new();
    for claim in extract_claims(text) {
        let Some(relatives) = resolve_scoped_paths(tree, &claim)
            .iter()
            .map(|path| tree.relative_to_root(path))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        let refutes = |relative: &String| {
            let truth = tree.truth(relative);
            let in_project = !claim.path.starts_with('/') && project_root.join(relative).exists();
            match claim.claim {
                Claim::Absent => truth.certainly_exists(),
                Claim::Exists => truth.certainly_absent() && !in_project,
            }
        };
        if relatives.is_empty() || !relatives.iter().all(refutes) {
            continue;
        }
        let (said, observed) = match claim.claim {
            Claim::Absent => (
                "does not exist",
                format!(
                    "it exists in repository {} at base commit {} and in the checkout",
                    tree.root().display(),
                    tree.base_commit()
                ),
            ),
            Claim::Exists => (
                "exists",
                format!(
                    "it is absent from repository {} at base commit {} and from the checkout",
                    tree.root().display(),
                    tree.base_commit()
                ),
            ),
        };
        let finding = format!(
            "{task_id}: the body says `{}` {said} (\"{}\") but {observed}; read the path under the repository root and rewrite the claim to what is there",
            relatives.join("` or `"),
            excerpt(&claim.sentence)
        );
        findings.push((relatives, finding));
    }
    findings
}

/// The paths a claim may be about. A path with components may already be
/// repository-relative, or it may be relative to the explicitly named
/// directory: keep an existing root path, otherwise use the only scoped
/// interpretation present in the tree. A bare name is joined to every
/// directory the scope may name. Empty when the claim names no path.
fn resolve_scoped_paths(tree: &RepositoryTree, claim: &PathClaim) -> Vec<String> {
    let Some(cited) = &claim.scope else {
        return vec![claim.path.clone()];
    };
    let scopes = scope_directories(tree, cited);
    if claim.scoped_join || !claim.path.contains('/') {
        return scopes
            .iter()
            .map(|scope| format!("{scope}/{}", claim.path))
            .collect();
    }
    if scopes.is_empty() {
        return Vec::new();
    }
    let Some(root_path) = tree.relative_to_root(&claim.path) else {
        return Vec::new();
    };
    if tree.truth(&root_path).certainly_exists() {
        return vec![claim.path.clone()];
    }
    let mut present = std::collections::BTreeSet::new();
    for scope in &scopes {
        let mut candidates = vec![format!("{scope}/{}", claim.path)];
        if let Some((scope_name, remainder)) = claim.path.split_once('/')
            && scope.rsplit('/').next() == Some(scope_name)
        {
            candidates.push(format!("{scope}/{remainder}"));
        }
        present.extend(
            candidates
                .into_iter()
                .filter(|path| tree.truth(path).certainly_exists()),
        );
    }
    if present.len() == 1 {
        present.into_iter().collect()
    } else {
        Vec::new()
    }
}

/// The repository directories an explicit scope may name. Empty when the
/// scope names a file (the token is then a symbol inside it, not a path).
///
/// - At the repository root the tree decides: a directory is the scope, a
///   file is no scope.
/// - Not at the root, the scope may be cited by base name or by a trailing
///   suffix (`spec.rs` or `src/spec.rs` for `crates/x/src/spec.rs`). Without
///   a trailing `/`, any base-commit file ending with it makes it a file.
///   Otherwise every base-commit directory ending with it is a candidate.
/// - Matching nothing at all: a scope spelled with a trailing `/`, or whose
///   last component has no file extension, is a missing directory and is
///   joined, so "`x.rs` exists under `docs/v1.3/`" is checked and refuted.
///   A scope like `config.json` may be a file outside the repository (a
///   project file, or one still to be written), so it is no scope.
fn scope_directories(tree: &RepositoryTree, cited: &str) -> Vec<String> {
    let slashed = cited.ends_with('/');
    let Some(scope) = tree.relative_to_root(cited).filter(|s| !s.is_empty()) else {
        return Vec::new();
    };
    if tree.is_dir_at_base(&scope) || tree.root().join(&scope).is_dir() {
        return vec![scope];
    }
    if tree.exists_at_base(&scope) || tree.root().join(&scope).is_file() {
        return Vec::new();
    }
    let suffix = format!("/{scope}");
    let (dirs, files): (Vec<&String>, Vec<&String>) = tree
        .paths_at_base()
        .iter()
        .filter(|path| path.ends_with(&suffix))
        .partition(|path| tree.is_dir_at_base(path));
    if !slashed && !files.is_empty() {
        return Vec::new();
    }
    if !dirs.is_empty() {
        return dirs.into_iter().cloned().collect();
    }
    let last = scope.rsplit('/').next().unwrap_or(&scope);
    if slashed || !parser::has_file_extension(last) {
        vec![scope]
    } else {
        Vec::new()
    }
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
