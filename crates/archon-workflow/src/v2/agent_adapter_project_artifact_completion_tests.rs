use std::path::Path;

use super::*;
use crate::{
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FileRecord, WorkflowV2HostMethod,
    WorkflowV2HostOptions,
};

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

#[test]
fn prose_artifact_requirement_is_not_resolved_as_project_path() {
    let temp = tempfile::tempdir().expect("tempdir").keep();
    let project = temp.join("project");
    let repo = temp.join("repo");
    let v2_root = project.join(".archon/workflows/wf-artifact-prose/v2");
    std::fs::create_dir_all(repo.join("src")).expect("repo");
    std::fs::create_dir_all(&v2_root).expect("v2");
    let prose = "Implementation evidence must include exact focused command output.";
    let mut request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "impl-artifact-prose".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "write implementation evidence".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({
            "item": {
                "canonical_task_ids": ["TASK-X-001"],
                "artifact_requirements": [prose]
            }
        }),
        repository_root: Some(repo.display().to_string()),
        project_artifacts: crate::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
    };
    request
        .project_artifacts
        .add_artifact_requirements(&request.input);
    let mut result = WorkflowV2Result::accepted("implementation complete");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "updated implementation source",
    ));
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/lib.rs"));
    let output = serde_json::to_string(&result).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("prose artifact guidance must not become a required path");

    assert_eq!(parsed.status, WorkflowV2Status::Accepted);
    assert!(parsed.data.get("missing_required_artifacts").is_none());
    assert!(
        parsed
            .artifacts
            .iter()
            .all(|artifact| !artifact.path.contains("Implementation evidence"))
    );
}

#[test]
fn glob_artifact_requirement_is_not_resolved_as_project_path() {
    let temp = tempfile::tempdir().expect("tempdir").keep();
    let project = temp.join("project");
    let repo = temp.join("repo");
    let v2_root = project.join(".archon/workflows/wf-artifact-glob/v2");
    std::fs::create_dir_all(repo.join("src")).expect("repo");
    std::fs::create_dir_all(&v2_root).expect("v2");
    let pattern = ".archon/trading-lab/data/datasets/*/*/validation.json";
    let mut request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "impl-artifact-glob".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "write implementation evidence".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({
            "item": {
                "canonical_task_ids": ["TASK-X-001"],
                "artifact_requirements": [pattern]
            }
        }),
        repository_root: Some(repo.display().to_string()),
        project_artifacts: crate::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
    };
    request
        .project_artifacts
        .add_artifact_requirements(&request.input);
    let mut result = WorkflowV2Result::accepted("implementation complete");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "updated implementation source",
    ));
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/lib.rs"));
    let output = serde_json::to_string(&result).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("glob artifact guidance must not become a required path");

    assert_eq!(parsed.status, WorkflowV2Status::Accepted);
    assert!(parsed.data.get("missing_required_artifacts").is_none());
}

#[test]
fn placeholder_artifact_requirement_is_not_resolved_as_project_path() {
    let temp = tempfile::tempdir().expect("tempdir").keep();
    let project = temp.join("project");
    let repo = temp.join("repo");
    let v2_root = project.join(".archon/workflows/wf-artifact-placeholder/v2");
    std::fs::create_dir_all(repo.join("src")).expect("repo");
    std::fs::create_dir_all(&v2_root).expect("v2");
    let placeholder = ".archon/data/datasets/<dataset-id>/<version>/validation.json";
    let mut request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "impl-artifact-placeholder".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "write implementation evidence".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({
            "item": {
                "canonical_task_ids": ["TASK-X-001"],
                "artifact_requirements": [{"path": placeholder}]
            }
        }),
        repository_root: Some(repo.display().to_string()),
        project_artifacts: crate::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
    };
    request
        .project_artifacts
        .add_artifact_requirements(&request.input);
    let mut result = WorkflowV2Result::accepted("implementation complete");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "updated implementation source",
    ));
    result
        .files_changed
        .push(WorkflowV2FileRecord::new("src/lib.rs"));

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &serde_json::to_string(&result).expect("json"))
        .expect("placeholder artifact guidance must not become a required path");

    assert_eq!(parsed.status, WorkflowV2Status::Accepted);
    assert!(parsed.data.get("missing_required_artifacts").is_none());
}

#[test]
fn namespaced_project_data_file_changed_is_artifact_evidence() {
    let temp = tempfile::tempdir().expect("tempdir").keep();
    let project = temp.join("project");
    let repo = temp.join("repo");
    let v2_root = project.join(".archon/workflows/wf-project-data/v2");
    std::fs::create_dir_all(repo.join("src")).expect("repo");
    std::fs::create_dir_all(&v2_root).expect("v2");
    let artifact_path = ".archon/provider-data/data/capabilities/latest.json";
    std::fs::create_dir_all(project.join(".archon/provider-data/data/capabilities"))
        .expect("artifact dir");
    std::fs::write(project.join(artifact_path), "{}").expect("artifact");
    let request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "impl-project-data-artifact".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "write provider data artifact".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: Some(repo.display().to_string()),
        project_artifacts: crate::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
    };
    let mut result = WorkflowV2Result::accepted("artifact written");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "created provider data artifact",
    ));
    result
        .files_changed
        .push(WorkflowV2FileRecord::new(artifact_path));
    let output = serde_json::to_string(&result).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("project data artifact must not be repo source ownership");

    assert_eq!(parsed.status, WorkflowV2Status::Accepted);
    assert!(parsed.files_changed.is_empty());
    assert_eq!(parsed.artifacts[0].path, artifact_path);
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
