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
        target_ownership_scopes: Vec::new(),
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
        target_ownership_scopes: Vec::new(),
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

#[test]
fn parser_coerces_unknown_command_kind_to_other() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let repo_root = repo.display().to_string();
    let request = write_request(&repo_root);
    let output = serde_json::json!({
        "status": "accepted",
        "summary": "changed native validation gate",
        "evidence": [{"kind": "implementation", "summary": "updated gate logic"}],
        "commands_run": [{
            "kind": "implementation",
            "command": "cargo test -p archon-trading validation_gates",
            "status": "succeeded",
            "exit_code": 0,
            "output_summary": "passed"
        }],
        "files_changed": [{"path": "crates/archon-trading/src/data_lake.rs"}]
    });

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output.to_string())
        .expect("unknown command kind should not reject result");

    assert_eq!(parsed.commands_run[0].kind, WorkflowV2CommandKind::Other);
}

#[test]
fn parser_normalizes_known_command_status_synonyms() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let request = write_request(&repo.display().to_string());
    let output = serde_json::json!({
        "status": "accepted",
        "summary": "changed native validation gate",
        "evidence": [{"kind": "implementation", "summary": "updated gate logic"}],
        "commands_run": [{
            "kind": "test",
            "command": "cargo test focused",
            "status": "passed",
            "exit_code": 0,
            "output_summary": "passed"
        }],
        "files_changed": [{"path": "crates/archon-trading/src/data_lake.rs"}]
    });

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output.to_string())
        .expect("known status synonym should normalize");

    assert_eq!(
        parsed.commands_run[0].status,
        WorkflowV2CommandStatus::Succeeded
    );
}

#[test]
fn write_prompt_ends_with_explicit_json_noop_contract() {
    let request = write_request("/repo");

    let prompt = WorkflowV2AgentAdapter::new().build_prompt(&request);

    assert!(prompt.contains("## Final Output Rule"));
    assert!(prompt.contains("Never return prose such as Status: noop"));
    assert!(prompt.trim_end().ends_with("Status: noop."));
}

#[test]
fn parser_stamps_missing_mechanical_artifact_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let repo_root = repo.display().to_string();
    let request = write_request(&repo_root);
    let output = serde_json::json!({
        "status": "accepted",
        "summary": "updated native validation gate",
        "evidence": [{"kind": "implementation", "summary": "updated gate logic"}],
        "artifacts": [{"path": "reports/validation.json"}],
        "commands_run": [{
            "command": "cargo test -p archon-trading validation_gates",
            "status": "succeeded",
            "exit_code": 0,
            "output_summary": "passed"
        }],
        "files_changed": [{"path": "crates/archon-trading/src/data_lake.rs"}]
    });

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output.to_string())
        .expect("mechanical fields are host stamped");

    assert_eq!(parsed.artifacts[0].id, "artifact-0-validation-json");
    assert_eq!(parsed.commands_run[0].kind, WorkflowV2CommandKind::Other);
}

#[test]
fn parser_coerces_bare_string_artifacts_and_file_records() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let request = write_request(&repo.display().to_string());
    let output = include_str!("fixtures/remediation_artifact_strings.json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, output)
        .expect("safe string records should normalize");

    assert_eq!(parsed.artifacts[0].path, "reports/validation.json");
    assert_eq!(
        parsed.files_read[0].path,
        "crates/archon-trading/src/data_lake.rs"
    );
    assert_eq!(
        parsed.files_changed[0].path,
        "crates/archon-trading/src/data_lake.rs"
    );
}

#[test]
fn parser_keeps_unknown_command_status_strict() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let request = write_request(&repo.display().to_string());
    let output = serde_json::json!({
        "status": "accepted",
        "summary": "changed native validation gate",
        "evidence": [{"kind": "implementation", "summary": "updated gate logic"}],
        "commands_run": [{
            "kind": "test",
            "command": "cargo test focused",
            "status": "maybe",
            "output_summary": "unknown"
        }],
        "files_changed": ["crates/archon-trading/src/data_lake.rs"]
    });

    assert!(
        WorkflowV2AgentAdapter::new()
            .parse_agent_output(&request, &output.to_string())
            .is_err()
    );
}

#[test]
fn string_changed_file_still_obeys_ownership() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let request = write_request(&repo.display().to_string());
    let output = serde_json::json!({
        "status": "accepted",
        "summary": "changed an undeclared file",
        "evidence": [{"kind": "implementation", "summary": "updated logic"}],
        "files_changed": ["crates/archon-trading/src/other.rs"]
    });

    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output.to_string())
        .expect_err("string coercion must not bypass ownership");

    assert!(matches!(
        error,
        WorkflowV2AgentError::ImplementationChangedFilesOutsideOwnership(_)
    ));
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

#[tokio::test]
async fn repair_allows_one_more_attempt_for_a_new_error_class() {
    let temp = tempfile::tempdir().expect("tempdir");
    let repo = temp.path().join("repo");
    std::fs::create_dir_all(repo.join("crates/archon-trading/src")).expect("repo");
    let repo_root = repo.display().to_string();
    let request = write_request(&repo_root);
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/compound_repair_outputs.json"))
            .expect("fixture");
    let corrected = serde_json::to_string(&accepted_result_with_absolute_change(&repo_root))
        .expect("result json");
    let client = SequenceClient {
        outputs: Mutex::new(vec![
            fixture["first"].as_str().unwrap().to_string(),
            fixture["repair"].to_string(),
            corrected,
        ]),
    };

    let parsed = WorkflowV2AgentAdapter::new()
        .run_with_repair(&client, &request)
        .await
        .expect("different repair class gets one bounded extra attempt");

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
    assert_eq!(
        context.branch_evidence_root.as_deref(),
        Some(v2_root.join("branches").to_string_lossy().as_ref())
    );
}

#[test]
fn accepted_branch_proof_is_discoverable_under_explicit_evidence_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    let v2_root = temp.path().join("project/.archon/workflows/wf-proof/v2");
    let proof = v2_root
        .join("branches/verification-wave-1")
        .join("verification-wave-1-verify-TASK-003-provider-proof.json");
    std::fs::create_dir_all(proof.parent().expect("proof parent")).expect("proof dir");
    std::fs::write(&proof, r#"{"status":"accepted"}"#).expect("proof");

    let context = crate::project_artifact_context_from_v2_root(&v2_root);
    let evidence_root = std::path::Path::new(
        context
            .branch_evidence_root
            .as_deref()
            .expect("branch evidence root"),
    );

    assert!(
        evidence_root
            .join("verification-wave-1")
            .join("verification-wave-1-verify-TASK-003-provider-proof.json")
            .exists()
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
        target_ownership_scopes: Vec::new(),
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
