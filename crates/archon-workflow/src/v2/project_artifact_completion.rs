//! Declared artifact contract enforcement.
//!
//! One contract source: the paths validated here are exactly the paths handed
//! to the agent — the same extraction that builds the prompt's resolved
//! artifact section (`project_artifact_prompt::declared_project_artifacts`)
//! drives validation, so an artifact can only be demanded if the agent was
//! instructed to produce it. No inference, no heuristic recovery of intent.
//!
//! A missing declared artifact produces a failed result VALUE for the call —
//! never a run-level block.
//!
//! # Issue #168: existence is not evidence
//!
//! This check used to ask `Path::exists()`. A directory exists. An empty file
//! exists. Run `wf-67dd2599` left directories in the project root named after
//! acceptance criteria, and a criterion-named directory answering an
//! existence check for the artifact that criterion describes is precisely the
//! shape of issue #153 — a contract reported satisfied by something containing
//! nothing. A declared artifact is now satisfied only by a regular, non-empty
//! file, and the reason a candidate failed is named in the failure.
//!
//! # Issue-12: structural emptiness is a review finding, not a failure
//!
//! A declared artifact that exists, has bytes, parses, and holds no records
//! (`artifact_emptiness::structurally_empty_defect`) used to fail the branch
//! exactly like a missing file. Observed live: a schema-migration branch
//! rewrote an empty v1 registry as an empty v2 registry — correct, because
//! populating it was a LATER task's job — and the authored script had listed
//! that registry under the item's `artifacts:`. The host failed the branch,
//! every dependent wave was skipped in the same second, and a genuine code
//! fix in the same branch was discarded with it.
//!
//! The host cannot know whether emptiness is legitimate for a given task; a
//! verifier reading the task's acceptance criteria can. So the signal is
//! kept — the artifact is recorded, and the branch gains a residual gap with
//! `severity: "review"` plus an evidence line naming the path — but it no
//! longer rewrites the branch's status. Missing, directory, and zero-byte
//! artifacts are unchanged: those are still hard failures. The policy is
//! hard-coded to "review": the adapter has no config hand-off, and threading
//! one through `WorkflowV2AgentAdapter` for a single boolean did not fit the
//! existing conventions cleanly.

use std::path::Path;

use super::artifact_path_guard::declared_artifact_defect;
use super::host_api::WorkflowV2ArtifactRequirement;
use super::project_artifact_prompt::declared_project_artifacts;
use super::{
    WorkflowV2Artifact, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2ProjectArtifactContext, WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2Status,
};

pub(super) fn enforce_declared_artifact_requirements(
    item_id: &str,
    input: &serde_json::Value,
    required_artifacts: &[WorkflowV2ArtifactRequirement],
    result: &mut WorkflowV2Result,
    context: &WorkflowV2ProjectArtifactContext,
) {
    if context.is_empty() {
        return;
    }
    let declared = declared_project_artifacts(input, required_artifacts, context);
    if declared.is_empty() {
        return;
    }
    let mut unsatisfied = Vec::new();
    let mut missing = Vec::new();
    let mut structurally_empty = Vec::new();
    for (raw, absolute) in &declared.entries {
        let project_path = Path::new(absolute);
        let satisfied_by =
            match declared_artifact_defect(raw, project_path, context.declared_as_directory(raw)) {
                None => Some(project_path.to_path_buf()),
                // Not under the project artifact root — try the repository. A
                // deliverable contract may name a source file, and source does not
                // live in the artifact tree. A live task was failed for
                // `data_store/coverage.rs (does not exist)` while that file sat in
                // the repository with 455 lines; passing would have meant writing
                // source into the artifact root, so no retry could have worked.
                //
                // Existence only. Nothing here grants a write anywhere: ownership
                // and confinement still answer to the project root alone.
                Some(defect) => match repository_candidate(raw, context) {
                    Some(candidate)
                        if declared_artifact_defect(
                            raw,
                            &candidate,
                            context.declared_as_directory(raw),
                        )
                        .is_none() =>
                    {
                        Some(candidate)
                    }
                    _ => {
                        missing.push(raw.clone());
                        unsatisfied.push(format!("{raw} ({defect})"));
                        None
                    }
                },
            };
        let Some(satisfied_by) = satisfied_by else {
            continue;
        };
        record_declared_artifact(result, raw);
        // The file is there and has bytes; whether it holds anything is a
        // separate question with a separate answer. A registry holding `{}`
        // once satisfied an existence check for a task that produced nothing
        // (see `artifact_emptiness`), so the signal is raised — but as review
        // data, because the same shape is exactly right for a schema or
        // scaffold task whose records a later task writes (issue-12).
        //
        // Applied HERE and not in `artifact_file_defect`, which answers the
        // broader question "is this file evidence" for callers that use `{}`
        // as ordinary placeholder content. This is the narrower case: an item
        // that DECLARED it would produce this artifact, and is now claiming
        // it did.
        if let Some(defect) = super::artifact_emptiness::structurally_empty_defect(&satisfied_by) {
            structurally_empty.push(format!("{raw} ({defect})"));
            result.evidence.push(WorkflowV2Evidence {
                kind: WorkflowV2EvidenceKind::Review,
                summary: format!(
                    "declared project artifact {raw} {defect}; legitimate only if this task's \
                     acceptance criteria do not require it to be populated"
                ),
                source: Some(raw.to_string()),
            });
        }
    }
    for (raw, reason) in &declared.refused {
        missing.push(raw.clone());
        unsatisfied.push(format!("{raw} (refused: {reason})"));
    }
    if !structurally_empty.is_empty() {
        review_structurally_empty_artifacts(item_id, result, &structurally_empty);
    }
    if unsatisfied.is_empty() {
        return;
    }
    fail_declared_artifact_contract(item_id, result, &missing, &unsatisfied);
}

/// Gap id prefix for a declared artifact that exists but holds no records.
pub const STRUCTURALLY_EMPTY_ARTIFACT_GAP_PREFIX: &str = "artifact_structurally_empty_";

/// Raise structural emptiness as a review gap the verifier and reviewers
/// see. Leaves `status` and `summary` alone: the branch keeps whatever it
/// earned, and a missing artifact found in the same pass still fails it.
fn review_structurally_empty_artifacts(
    item_id: &str,
    result: &mut WorkflowV2Result,
    structurally_empty: &[String],
) {
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "{STRUCTURALLY_EMPTY_ARTIFACT_GAP_PREFIX}{}",
            sanitize_gap_id(item_id)
        ),
        description: format!(
            "declared project artifacts for '{item_id}' exist but hold no records: {}; \
             verify against the task's acceptance criteria whether this task was \
             required to populate them",
            structurally_empty.join(", ")
        ),
        severity: Some("review".to_string()),
    });
}

fn fail_declared_artifact_contract(
    item_id: &str,
    result: &mut WorkflowV2Result,
    missing: &[String],
    unsatisfied: &[String],
) {
    result.status = WorkflowV2Status::Failed;
    result.summary = format!(
        "declared project artifacts missing for '{item_id}': {}",
        unsatisfied.join(", ")
    );
    if let serde_json::Value::Object(data) = &mut result.data {
        data.insert(
            "missing_required_artifacts".to_string(),
            serde_json::json!(missing),
        );
    } else if result.data.is_null() {
        result.data = serde_json::json!({ "missing_required_artifacts": missing });
    }
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("missing_declared_artifacts_{}", sanitize_gap_id(item_id)),
        description: format!(
            "declared artifact contract not satisfied; missing: {}",
            unsatisfied.join(", ")
        ),
        severity: Some("failed".to_string()),
    });
}

/// The same relative path resolved under the repository root, when one is
/// configured and differs from the project root.
fn repository_candidate(
    raw: &str,
    context: &WorkflowV2ProjectArtifactContext,
) -> Option<std::path::PathBuf> {
    let repo = context
        .repository_root
        .as_deref()
        .filter(|root| !root.is_empty())?;
    if context.project_root.as_deref() == Some(repo) {
        return None;
    }
    let relative = raw.trim_start_matches('/');
    if relative.is_empty() || relative.contains("..") {
        return None;
    }
    Some(Path::new(repo).join(relative))
}

fn record_declared_artifact(result: &mut WorkflowV2Result, path: &str) {
    if !result
        .artifacts
        .iter()
        .any(|existing| existing.path == path)
    {
        result.artifacts.push(WorkflowV2Artifact {
            id: artifact_id(path),
            path: path.to_string(),
            description: Some("declared project artifact".to_string()),
        });
    }
    result.evidence.push(WorkflowV2Evidence {
        kind: WorkflowV2EvidenceKind::Artifact,
        summary: format!("existing required project artifact: {path}"),
        source: Some(path.to_string()),
    });
}

fn artifact_id(path: &str) -> String {
    let id = path
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    id.trim_matches('-').to_string()
}

fn sanitize_gap_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod repository_fallback_tests {
    use super::repository_candidate;
    use crate::v2::project_artifacts::WorkflowV2ProjectArtifactContext;

    fn context(project: &str, repo: Option<&str>) -> WorkflowV2ProjectArtifactContext {
        WorkflowV2ProjectArtifactContext {
            project_root: Some(project.to_string()),
            repository_root: repo.map(str::to_string),
            ..Default::default()
        }
    }

    /// The live case: a source deliverable lives in the repository, not the
    /// artifact tree, so the check must be able to look there.
    #[test]
    fn a_source_path_resolves_under_the_repository() {
        let c = context("/work/project-1", Some("/work/archon-cli"));
        assert_eq!(
            repository_candidate("crates/archon-trading/src/data_store/coverage.rs", &c),
            Some(std::path::PathBuf::from(
                "/work/archon-cli/crates/archon-trading/src/data_store/coverage.rs"
            ))
        );
    }

    /// One tree: there is no second place to look, so nothing is offered.
    #[test]
    fn an_identical_root_offers_no_second_candidate() {
        let c = context("/work/repo", Some("/work/repo"));
        assert_eq!(repository_candidate("src/lib.rs", &c), None);
    }

    #[test]
    fn without_a_repository_root_nothing_is_offered() {
        let c = context("/work/project-1", None);
        assert_eq!(repository_candidate("src/lib.rs", &c), None);
    }

    /// Traversal is refused rather than resolved — the fallback must not become
    /// a way to point at anything outside the repository.
    #[test]
    fn traversal_is_refused() {
        let c = context("/work/project-1", Some("/work/archon-cli"));
        assert_eq!(repository_candidate("../../etc/passwd", &c), None);
        assert_eq!(repository_candidate("a/../../b", &c), None);
    }
}
