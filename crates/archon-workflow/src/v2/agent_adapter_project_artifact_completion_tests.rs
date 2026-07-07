use std::path::Path;

use super::*;
use crate::{WorkflowV2EvidenceKind, WorkflowV2HostMethod, WorkflowV2HostOptions};

#[test]
fn declared_artifact_present_is_recorded_verbatim_as_evidence() {
    let (request, artifact_path) = request_with_required_artifact("impl-artifact-complete");
    write_project_artifact_file(&request, &artifact_path);
    let output = serde_json::to_string(&WorkflowV2Result::accepted("artifact written"))
        .expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("existing declared artifact completes evidence envelope");

    assert_eq!(parsed.status, WorkflowV2Status::Accepted);
    assert_eq!(parsed.artifacts[0].path, artifact_path);
    assert!(parsed.evidence.iter().any(|evidence| {
        evidence.kind == WorkflowV2EvidenceKind::Artifact
            && evidence
                .summary
                .contains("existing required project artifact")
    }));
}

#[test]
fn missing_declared_artifact_is_a_failed_result_value() {
    let (request, artifact_path) = request_with_required_artifact("impl-artifact-missing");
    let output = serde_json::to_string(&WorkflowV2Result::accepted("artifact written"))
        .expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("missing declared artifact is a failed result value, not an error");

    assert_eq!(parsed.status, WorkflowV2Status::Failed);
    assert!(parsed.artifacts.is_empty());
    assert!(
        parsed.data["missing_required_artifacts"]
            .as_array()
            .is_some_and(|missing| missing.iter().any(|path| path == &artifact_path))
    );
    assert!(parsed.residual_gaps.iter().any(|gap| {
        gap.description
            .contains("declared artifact contract not satisfied")
            && gap.description.contains(&artifact_path)
    }));
}

fn request_with_required_artifact(call_id: &str) -> (WorkflowV2AgentRequest, String) {
    let temp = tempfile::tempdir().expect("tempdir").keep();
    let project = temp.join("project");
    let repo = temp.join("repo");
    let v2_root = project.join(".archon/workflows/wf-artifact-complete/v2");
    std::fs::create_dir_all(repo.join("src")).expect("repo");
    std::fs::create_dir_all(&v2_root).expect("v2");
    let artifact_path = format!(".archon/workflows/wf-artifact-complete/artifacts/{call_id}.md");
    let mut request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: call_id.to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "write required project artifact".to_string(),
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
    };
    request
        .project_artifacts
        .add_artifact_requirements(&request.input);
    (request, artifact_path)
}

fn write_project_artifact_file(request: &WorkflowV2AgentRequest, path: &str) {
    let root = request
        .project_artifacts
        .project_root
        .as_deref()
        .expect("project root");
    let absolute = Path::new(root).join(path);
    std::fs::create_dir_all(absolute.parent().expect("artifact parent")).expect("artifact dir");
    std::fs::write(absolute, "# artifact evidence").expect("artifact");
}
