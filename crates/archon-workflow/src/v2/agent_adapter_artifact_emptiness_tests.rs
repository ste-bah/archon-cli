//! Issue-12: a declared artifact that exists and parses but holds no records
//! is a review finding on the branch, not a branch failure.
//!
//! Live, a schema-migration branch rewrote an empty v1 registry as an empty
//! v2 registry — correct, because population belonged to a later task — and
//! the host failed it for `exists but holds no records`, skipping every
//! dependent wave and discarding a genuine code fix in the same branch. The
//! host cannot know whether emptiness is legitimate; the verifier can. So the
//! signal survives as a `review` gap and an evidence line, while missing and
//! zero-byte artifacts stay the hard failures they were.

use std::path::Path;

use super::*;
use crate::v2::project_artifact_completion::STRUCTURALLY_EMPTY_ARTIFACT_GAP_PREFIX;
use crate::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2HostMethod,
    WorkflowV2HostOptions,
};

const EMPTY_REGISTRY: &str = r#"{"schema": "registry-v2", "items": []}"#;
const EMPTINESS_SENTENCE: &str =
    "exists but holds no records: every array and object in it is empty";

/// The live shape: an accepted branch with a real code change whose declared
/// artifact is a valid, empty registry. Status unchanged, artifact recorded,
/// review gap and evidence present, and none of the failure markers.
#[test]
fn a_structurally_empty_declared_artifact_keeps_the_accepted_status_and_adds_a_review_gap() {
    let (request, artifact_path) = request_with_required_artifact("impl-empty-registry");
    write_project_artifact_file(&request, &artifact_path, EMPTY_REGISTRY);
    let output = serde_json::to_string(&accepted_with_code_change()).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("a legitimately empty artifact is not a failed result value");

    assert_eq!(parsed.status, WorkflowV2Status::Accepted, "{parsed:#?}");
    assert_eq!(parsed.summary, "migrated the registry schema");
    assert!(
        parsed.data.get("missing_required_artifacts").is_none(),
        "{:#?}",
        parsed.data
    );
    assert!(
        !parsed
            .summary
            .contains("declared project artifacts missing"),
        "{}",
        parsed.summary
    );
    assert!(
        parsed
            .artifacts
            .iter()
            .any(|artifact| artifact.path == artifact_path),
        "the artifact exists and is recorded as delivered: {:#?}",
        parsed.artifacts
    );
    let gap = parsed
        .residual_gaps
        .iter()
        .find(|gap| {
            gap.id == format!("{STRUCTURALLY_EMPTY_ARTIFACT_GAP_PREFIX}impl-empty-registry")
        })
        .unwrap_or_else(|| panic!("no review gap: {:#?}", parsed.residual_gaps));
    assert_eq!(gap.severity.as_deref(), Some("review"));
    assert!(
        gap.description.contains(&artifact_path),
        "{}",
        gap.description
    );
    assert!(
        gap.description.contains(EMPTINESS_SENTENCE),
        "{}",
        gap.description
    );
    assert!(
        !parsed
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with("missing_declared_artifacts_")),
        "{:#?}",
        parsed.residual_gaps
    );
    assert!(
        parsed.evidence.iter().any(|evidence| {
            evidence.kind == WorkflowV2EvidenceKind::Review
                && evidence.summary.contains(&artifact_path)
                && evidence.summary.contains(EMPTINESS_SENTENCE)
                && evidence.source.as_deref() == Some(artifact_path.as_str())
        }),
        "the verifier must see the emptiness as an evidence line: {:#?}",
        parsed.evidence
    );
}

/// The rule's origin case must not regress into silence: a bare `{}` is
/// still flagged, only no longer as a failure.
#[test]
fn a_bare_empty_object_is_flagged_for_review_not_failed() {
    let (request, artifact_path) = request_with_required_artifact("impl-empty-object");
    write_project_artifact_file(&request, &artifact_path, "{}");
    let output = serde_json::to_string(&accepted_with_code_change()).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("accepted");

    assert_eq!(parsed.status, WorkflowV2Status::Accepted, "{parsed:#?}");
    assert!(
        parsed.residual_gaps.iter().any(|gap| gap
            .id
            .starts_with(STRUCTURALLY_EMPTY_ARTIFACT_GAP_PREFIX)
            && gap.severity.as_deref() == Some("review")),
        "{:#?}",
        parsed.residual_gaps
    );
}

/// A populated artifact raises nothing: the gap is for emptiness only.
#[test]
fn a_populated_declared_artifact_raises_no_review_gap() {
    let (request, artifact_path) = request_with_required_artifact("impl-populated");
    write_project_artifact_file(
        &request,
        &artifact_path,
        r#"{"schema": "registry-v2", "items": [{"id": "one"}]}"#,
    );
    let output = serde_json::to_string(&accepted_with_code_change()).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("accepted");

    assert_eq!(parsed.status, WorkflowV2Status::Accepted);
    assert!(
        !parsed
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with(STRUCTURALLY_EMPTY_ARTIFACT_GAP_PREFIX)),
        "{:#?}",
        parsed.residual_gaps
    );
    assert!(
        !parsed
            .evidence
            .iter()
            .any(|evidence| evidence.summary.contains(EMPTINESS_SENTENCE)),
        "{:#?}",
        parsed.evidence
    );
}

/// Unchanged: a declared artifact that is not on disk still fails the branch.
#[test]
fn a_missing_declared_artifact_still_fails_the_branch() {
    let (request, artifact_path) = request_with_required_artifact("impl-still-missing");
    let output = serde_json::to_string(&accepted_with_code_change()).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("missing declared artifact is a failed result value, not an error");

    assert_eq!(parsed.status, WorkflowV2Status::Failed, "{parsed:#?}");
    assert!(
        parsed
            .summary
            .contains("declared project artifacts missing"),
        "{}",
        parsed.summary
    );
    assert!(
        parsed.summary.contains("does not exist"),
        "{}",
        parsed.summary
    );
    assert!(
        parsed.data["missing_required_artifacts"]
            .as_array()
            .is_some_and(|missing| missing.iter().any(|path| path == &artifact_path))
    );
    assert!(
        parsed.residual_gaps.iter().any(|gap| gap.id
            == "missing_declared_artifacts_impl-still-missing"
            && gap.severity.as_deref() == Some("failed")),
        "{:#?}",
        parsed.residual_gaps
    );
    assert!(
        !parsed
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with(STRUCTURALLY_EMPTY_ARTIFACT_GAP_PREFIX)),
        "a missing file is not 'empty': {:#?}",
        parsed.residual_gaps
    );
}

/// Unchanged: a zero-byte declared artifact still fails the branch. Bytes
/// that parse to nothing are review data; no bytes at all is no artifact.
#[test]
fn a_zero_byte_declared_artifact_still_fails_the_branch() {
    let (request, artifact_path) = request_with_required_artifact("impl-zero-byte");
    write_project_artifact_file(&request, &artifact_path, "");
    let output = serde_json::to_string(&accepted_with_code_change()).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("zero-byte declared artifact is a failed result value, not an error");

    assert_eq!(parsed.status, WorkflowV2Status::Failed, "{parsed:#?}");
    assert!(
        parsed.summary.contains("is an empty file"),
        "{}",
        parsed.summary
    );
    assert!(
        parsed.data["missing_required_artifacts"]
            .as_array()
            .is_some_and(|missing| missing.iter().any(|path| path == &artifact_path))
    );
    assert!(
        !parsed
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with(STRUCTURALLY_EMPTY_ARTIFACT_GAP_PREFIX)),
        "{:#?}",
        parsed.residual_gaps
    );
}

/// Two declared artifacts, one missing and one empty: the missing one still
/// fails the branch, and the empty one is still reported for review beside
/// it rather than being folded into the failure.
#[test]
fn a_missing_sibling_still_fails_while_the_empty_one_is_reviewed() {
    let (mut request, empty_path) = request_with_required_artifact("impl-mixed");
    let missing_path = ".archon/workflows/wf-artifact-complete/artifacts/impl-mixed-missing.md";
    request.input = serde_json::json!({
        "item": {
            "canonical_task_ids": ["TASK-X-001"],
            "artifact_requirements": [{"path": empty_path}, {"path": missing_path}]
        }
    });
    request
        .project_artifacts
        .add_artifact_requirements(&request.input);
    write_project_artifact_file(&request, &empty_path, EMPTY_REGISTRY);
    let output = serde_json::to_string(&accepted_with_code_change()).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("failed result value");

    assert_eq!(parsed.status, WorkflowV2Status::Failed, "{parsed:#?}");
    assert!(parsed.summary.contains(missing_path), "{}", parsed.summary);
    assert!(
        !parsed.summary.contains(&empty_path),
        "the empty artifact is not what failed the branch: {}",
        parsed.summary
    );
    assert!(
        parsed.residual_gaps.iter().any(|gap| gap
            .id
            .starts_with(STRUCTURALLY_EMPTY_ARTIFACT_GAP_PREFIX)
            && gap.description.contains(&empty_path)),
        "{:#?}",
        parsed.residual_gaps
    );
}

fn accepted_with_code_change() -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("migrated the registry schema");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "rewrote the schema module and its migration",
    ));
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/lib.rs"));
    result
}

fn request_with_required_artifact(call_id: &str) -> (WorkflowV2AgentRequest, String) {
    let temp = tempfile::tempdir().expect("tempdir").keep();
    let project = temp.join("project");
    let repo = temp.join("repo");
    let v2_root = project.join(".archon/workflows/wf-artifact-complete/v2");
    std::fs::create_dir_all(repo.join("src")).expect("repo");
    std::fs::create_dir_all(&v2_root).expect("v2");
    let artifact_path = format!(".archon/workflows/wf-artifact-complete/artifacts/{call_id}.json");
    let mut request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: call_id.to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "migrate the registry schema".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({
            "item": {
                "canonical_task_ids": ["TASK-X-001"],
                "artifact_requirements": [{"path": artifact_path}]
            }
        }),
        repository_root: Some(repo.display().to_string()),
        project_artifacts: crate::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
        target_ownership_scopes: Vec::new(),
    };
    request
        .project_artifacts
        .add_artifact_requirements(&request.input);
    (request, artifact_path)
}

fn write_project_artifact_file(request: &WorkflowV2AgentRequest, path: &str, body: &str) {
    let root = request
        .project_artifacts
        .project_root
        .as_deref()
        .expect("project root");
    let absolute = Path::new(root).join(path);
    std::fs::create_dir_all(absolute.parent().expect("artifact parent")).expect("artifact dir");
    std::fs::write(absolute, body).expect("artifact");
}
