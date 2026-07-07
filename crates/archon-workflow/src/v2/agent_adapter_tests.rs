use std::path::Path;
use std::sync::Mutex;

use super::*;
use crate::{
    WorkflowV2Artifact, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2FileRecord,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2TaskCoverage,
    WorkflowV2TaskCoverageStatus,
};

fn write_request(repo_root: &str) -> WorkflowV2AgentRequest {
    WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "implementation-wave-2-impl-TASK-TDL-020-validation-native-gates".to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "Implement TASK-TDL-020".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: Some(repo_root.to_string()),
        project_artifacts: Default::default(),
        target_files: vec!["crates/archon-trading/src/data_lake.rs".to_string()],
    }
}

#[test]
fn build_prompt_resolves_project_artifacts_under_project_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let project = temp.path().join("project");
    let repo = temp.path().join("repo");
    let v2_root = project.join(".archon/workflows/wf-artifact/v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");
    std::fs::create_dir_all(&repo).expect("repo");
    let mut request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: "verification-wave".to_string(),
            method: WorkflowV2HostMethod::Parallel,
            write_mode: None,
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "verify project artifact".to_string(),
        constraints: Vec::new(),
        input: serde_json::json!({
            "artifact_requirements": [
                {"path": ".archon/artifacts/required.json", "root": "projectArtifactRoot"}
            ]
        }),
        repository_root: Some(repo.display().to_string()),
        project_artifacts: crate::project_artifact_context_from_v2_root(&v2_root),
        target_files: Vec::new(),
    };
    request
        .project_artifacts
        .add_artifact_requirements(&request.input);

    let prompt = WorkflowV2AgentAdapter::new().build_prompt(&request);
    let expected = project.join(".archon/artifacts/required.json");

    assert!(prompt.contains("Resolved Project Artifact Paths"));
    assert!(prompt.contains(&expected.display().to_string()));
    assert!(prompt.contains(
        "Do not resolve relative `.archon/...` artifact paths against `repository_root`"
    ));
}

fn accepted_result_with_absolute_change(repo_root: &str) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::accepted("changed native validation gate");
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Implementation,
        "updated native validation gate logic",
    ));
    result.files_changed.push(WorkflowV2FileRecord::new(
        Path::new(repo_root)
            .join("crates/archon-trading/src/data_lake.rs")
            .display()
            .to_string(),
    ));
    result
}

#[test]
fn parser_canonicalizes_absolute_in_repo_changed_file_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let repo_root = repo.display().to_string();
    let request = write_request(&repo_root);
    let output = serde_json::to_string(&accepted_result_with_absolute_change(&repo_root))
        .expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("parse");

    assert_eq!(
        parsed.files_changed[0].path,
        "crates/archon-trading/src/data_lake.rs"
    );
}

struct SequenceClient {
    outputs: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl WorkflowV2AgentClient for SequenceClient {
    async fn run_agent_request(
        &self,
        _request: &WorkflowV2AgentRequest,
        _prompt: String,
    ) -> Result<String, WorkflowV2AgentError> {
        let mut outputs = self.outputs.lock().expect("outputs lock");
        if outputs.is_empty() {
            return Err(WorkflowV2AgentError::Transport(
                "test client exhausted".to_string(),
            ));
        }
        Ok(outputs.remove(0))
    }

    async fn run_agent(&self, _prompt: String) -> Result<String, WorkflowV2AgentError> {
        unreachable!("tests use run_agent_request")
    }
}

#[tokio::test]
async fn repair_response_uses_repository_aware_ownership_validation() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let repo_root = repo.display().to_string();
    let request = write_request(&repo_root);
    let repaired = serde_json::to_string(&accepted_result_with_absolute_change(&repo_root))
        .expect("result json");
    let client = SequenceClient {
        outputs: Mutex::new(vec!["Do you want me to proceed?".to_string(), repaired]),
    };

    let parsed = WorkflowV2AgentAdapter::new()
        .run_with_repair(&client, &request)
        .await
        .expect("repair accepted");

    assert_eq!(
        parsed.files_changed[0].path,
        "crates/archon-trading/src/data_lake.rs"
    );
}

#[test]
fn project_artifact_context_has_no_workflow_specific_default_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_root = temp.path().join("project/.archon/workflows/wf-generic/v2");
    std::fs::create_dir_all(&v2_root).expect("v2 root");

    let context = crate::project_artifact_context_from_v2_root(&v2_root);

    assert!(
        context
            .artifact_roots
            .iter()
            .all(|root| !root.contains("trading-lab")),
        "artifact roots must come from workflow context or declared requirements"
    );
}

#[test]
fn artifact_declaring_noop_requires_existing_project_artifact_evidence() {
    let (mut request, _) = project_artifact_request("implementation-wave-5-impl-a");
    add_generic_artifact_requirement(&mut request, ".archon/artifacts/required.json");
    let output = serde_json::to_string(&artifact_noop_result(None)).expect("json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("missing declared artifact is a failed result value, not an error");

    assert_eq!(parsed.status, WorkflowV2Status::Failed);
    assert!(parsed.artifacts.is_empty());
    assert!(
        parsed.data["missing_required_artifacts"]
            .as_array()
            .is_some_and(|missing| missing
                .iter()
                .any(|path| path == ".archon/artifacts/required.json"))
    );
    assert!(parsed.residual_gaps.iter().any(|gap| {
        gap.description
            .contains("declared artifact contract not satisfied")
            && gap.description.contains(".archon/artifacts/required.json")
    }));
}

#[test]
fn artifact_declaring_noop_accepts_existing_project_artifact_evidence() {
    let (mut request, _) = project_artifact_request("implementation-wave-6-impl-a");
    let artifact_path = ".archon/artifacts/required.json";
    add_generic_artifact_requirement(&mut request, artifact_path);
    write_project_artifact_file(&request, artifact_path);
    let output = serde_json::to_string(&artifact_noop_result(Some(artifact_path))).expect("json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("existing artifact evidence satisfies artifact-producing noop");

    assert_eq!(parsed.status, WorkflowV2Status::Noop);
    assert_eq!(parsed.artifacts[0].path, artifact_path);
}

fn project_artifact_request(call_id: &str) -> (WorkflowV2AgentRequest, String) {
    let temp = tempfile::tempdir().expect("tempdir").keep();
    let project = temp.join("project");
    let repo = temp.join("repo");
    let v2_root = project.join(".archon/workflows/wf-artifact-result/v2");
    std::fs::create_dir_all(repo.join("src")).expect("repo");
    std::fs::create_dir_all(&v2_root).expect("v2");
    let artifact_path = format!(
        ".archon/workflows/wf-artifact-result/artifacts/{}/branch-result.json",
        call_id.split("-impl").next().unwrap_or(call_id)
    );
    let request = WorkflowV2AgentRequest {
        call: WorkflowV2HostCall {
            id: call_id.to_string(),
            method: WorkflowV2HostMethod::Implementation,
            write_mode: Some(WorkflowV2WriteMode::Coordinated),
            options: WorkflowV2HostOptions::default(),
        },
        role: "coder".to_string(),
        task: "write typed branch result artifact".to_string(),
        constraints: Vec::new(),
        input: serde_json::Value::Null,
        repository_root: Some(repo.display().to_string()),
        project_artifacts: crate::project_artifact_context_from_v2_root(&v2_root),
        target_files: vec!["src/lib.rs".to_string()],
    };
    (request, artifact_path)
}

fn add_generic_artifact_requirement(request: &mut WorkflowV2AgentRequest, path: &str) {
    request.input = serde_json::json!({ "artifact_requirements": [path] });
    request
        .project_artifacts
        .add_artifact_requirements(&request.input);
}

fn artifact_noop_result(artifact_path: Option<&str>) -> WorkflowV2Result {
    let mut result = WorkflowV2Result::noop("artifact work already complete");
    result.task_coverage.push(WorkflowV2TaskCoverage {
        task_id: "TASK-X-001".to_string(),
        status: WorkflowV2TaskCoverageStatus::Noop,
        summary: "task already complete".to_string(),
        evidence: vec![WorkflowV2Evidence::new(
            WorkflowV2EvidenceKind::Implementation,
            "typed noop evidence",
        )],
    });
    if let Some(path) = artifact_path {
        result.artifacts.push(WorkflowV2Artifact {
            id: "required".to_string(),
            path: path.to_string(),
            description: None,
        });
    }
    result
}

fn write_project_artifact_file(request: &WorkflowV2AgentRequest, path: &str) {
    let root = request
        .project_artifacts
        .project_root
        .as_deref()
        .expect("project root");
    let absolute = Path::new(root).join(path);
    std::fs::create_dir_all(absolute.parent().expect("artifact parent")).expect("artifact dir");
    std::fs::write(absolute, "{}").expect("artifact");
}

fn write_branch_result_artifact(request: &WorkflowV2AgentRequest, artifact_path: &str) {
    let root = request
        .project_artifacts
        .project_root
        .as_deref()
        .expect("project root");
    let absolute = Path::new(root).join(artifact_path);
    std::fs::create_dir_all(absolute.parent().expect("artifact parent")).expect("artifact dir");
    let result = serde_json::json!({
        "schema": "workflow_v2_branch_result",
        "status": "accepted",
        "summary": "typed project artifact branch result",
        "item_id": request.call.id,
        "canonical_task_ids": ["TASK-X-001"],
        "evidence": [{
            "kind": "implementation",
            "summary": "typed artifact result is concrete evidence"
        }],
        "task_coverage": [WorkflowV2TaskCoverage {
            task_id: "TASK-X-001".to_string(),
            status: WorkflowV2TaskCoverageStatus::Accepted,
            summary: "task accepted by typed artifact result".to_string(),
            evidence: vec![WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Implementation,
                "typed artifact branch result",
            )],
        }]
    });
    std::fs::write(absolute, serde_json::to_string(&result).expect("json")).expect("artifact");
}

fn write_schema_less_evidence_artifact(
    request: &WorkflowV2AgentRequest,
    artifact_path: &str,
    command_passed: bool,
) {
    let root = request
        .project_artifacts
        .project_root
        .as_deref()
        .expect("project root");
    let absolute = Path::new(root).join(artifact_path);
    std::fs::create_dir_all(absolute.parent().expect("artifact parent")).expect("artifact dir");
    let command_status = if command_passed {
        "succeeded"
    } else {
        "failed"
    };
    let exit_code = if command_passed { 0 } else { 1 };
    let result = serde_json::json!({
        "final_status": "pass",
        "summary": "schema-less project artifact evidence",
        "item_id": request.call.id,
        "canonical_task_ids": ["TASK-X-001"],
        "commands": [{
            "kind": "test",
            "command": "lint check",
            "status": command_status,
            "exit_code": exit_code,
            "output_summary": "lint check output"
        }],
        "task_coverage": [{
            "task_id": "TASK-X-001",
            "status": "accepted",
            "summary": "task accepted by artifact evidence",
            "evidence": [{
                "kind": "implementation",
                "summary": "schema-less artifact branch result"
            }]
        }]
    });
    std::fs::write(absolute, serde_json::to_string(&result).expect("json")).expect("artifact");
}
