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
        target_ownership_scopes: Vec::new(),
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
        target_ownership_scopes: Vec::new(),
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
        target_ownership_scopes: Vec::new(),
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
        target_ownership_scopes: Vec::new(),
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

/// A write-capable result that names an artifact not on disk is a false
/// report the agent must repair, so the adapter raises it as an error the
/// bounded repair loop re-asks about, naming the path, instead of leaving a
/// blocking gap nobody downstream re-asks.
#[test]
fn declared_artifact_absent_from_disk_is_a_repairable_error() {
    let (request, artifact_path) = request_with_required_artifact("impl-artifact-claimed");
    let claimed = ".archon/workflows/wf-artifact-complete/artifacts/never-written.json";
    let mut result = WorkflowV2Result::accepted("artifact written");
    result.artifacts.push(crate::WorkflowV2Artifact {
        id: "claimed".to_string(),
        path: claimed.to_string(),
        description: None,
    });
    write_project_artifact_file(&request, &artifact_path);
    let output = serde_json::to_string(&result).expect("result json");

    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect_err("a declared artifact that does not exist must be re-asked, not inherited");

    match &error {
        WorkflowV2AgentError::DeclaredArtifactAbsent(paths) => {
            assert_eq!(paths.len(), 1, "{paths:?}");
            assert!(paths[0].contains("never-written.json"), "{paths:?}");
            assert!(paths[0].contains("does not exist"), "{paths:?}");
        }
        other => panic!("expected DeclaredArtifactAbsent, got {other:?}"),
    }
    assert!(
        error
            .to_string()
            .contains("honest blocked or failed status"),
        "{error}"
    );

    write_project_artifact_file(&request, claimed);
    WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("the same claim is accepted once the artifact exists");
}

/// The gap namespace belongs to the host: an agent that pre-seeds a
/// `missing_project_artifact_*` gap for its own absent claim is still re-asked.
#[test]
fn a_pre_seeded_missing_artifact_gap_does_not_dodge_the_error() {
    let (request, artifact_path) = request_with_required_artifact("impl-artifact-preseeded");
    write_project_artifact_file(&request, &artifact_path);
    let claimed = ".archon/workflows/wf-artifact-complete/artifacts/never-written.json";
    let mut result = WorkflowV2Result::accepted("artifact written");
    result.artifacts.push(crate::WorkflowV2Artifact {
        id: "claimed".to_string(),
        path: claimed.to_string(),
        description: None,
    });
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: host_missing_artifact_gap_id(claimed),
        description: "already known".to_string(),
        severity: Some("blocking".to_string()),
    });
    let output = serde_json::to_string(&result).expect("result json");

    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect_err("the host namespace is not the agent's to fill");
    assert!(
        matches!(&error, WorkflowV2AgentError::DeclaredArtifactAbsent(paths) if paths.len() == 1),
        "{error:?}"
    );
}

/// An honest blocked result that names what it could not produce is review
/// data, not a false report: the gap stays and no repair budget is spent.
#[test]
fn a_blocked_result_naming_an_absent_artifact_keeps_the_gap() {
    let (request, artifact_path) = request_with_required_artifact("impl-artifact-blocked");
    write_project_artifact_file(&request, &artifact_path);
    let claimed = ".archon/workflows/wf-artifact-complete/artifacts/never-written.json";
    let mut result = WorkflowV2Result::accepted("could not finish");
    result.status = WorkflowV2Status::Blocked;
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: "upstream-unavailable".to_string(),
        description: "the input this artifact is built from was not available".to_string(),
        severity: Some("blocking".to_string()),
    });
    result.artifacts.push(crate::WorkflowV2Artifact {
        id: "claimed".to_string(),
        path: claimed.to_string(),
        description: None,
    });
    let output = serde_json::to_string(&result).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("an honest stop is not re-asked");
    assert_eq!(parsed.status, WorkflowV2Status::Blocked);
    assert!(parsed.residual_gaps.iter().any(|gap| {
        gap.id.starts_with("missing_project_artifact_") && gap.description.contains(claimed)
    }));
}

/// A false claim is re-asked on the status the agent returned, not on the one
/// a host step rewrote: a required artifact missing turns the result `failed`,
/// and that must not shield a second, unrequired claim from repair.
#[test]
fn a_host_rewritten_status_does_not_shield_a_false_claim() {
    let (request, _required) = request_with_required_artifact("impl-artifact-shielded");
    let claimed = ".archon/workflows/wf-artifact-complete/artifacts/never-written.json";
    let mut result = WorkflowV2Result::accepted("both written");
    result.artifacts.push(crate::WorkflowV2Artifact {
        id: "claimed".to_string(),
        path: claimed.to_string(),
        description: None,
    });
    let output = serde_json::to_string(&result).expect("result json");

    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect_err("the agent said accepted; the host's failed rewrite is not an honest stop");
    assert!(
        matches!(&error, WorkflowV2AgentError::DeclaredArtifactAbsent(paths)
            if paths.iter().any(|path| path.starts_with(claimed))),
        "{error:?}"
    );
}

/// A claim the normalizer rewrites (`./` prefix) is still the agent's claim:
/// the error names it as written, so no spelling of a path escapes repair.
#[test]
fn a_dot_prefixed_absent_claim_is_still_re_asked() {
    let (request, artifact_path) = request_with_required_artifact("impl-artifact-dotted");
    write_project_artifact_file(&request, &artifact_path);
    let claimed = "./.archon/workflows/wf-artifact-complete/artifacts/never-written.json";
    let mut result = WorkflowV2Result::accepted("artifact written");
    result
        .files_changed
        .push(WorkflowV2FileRecord::new(claimed));
    let output = serde_json::to_string(&result).expect("result json");

    let error = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect_err("a rewritten spelling of an absent path is still a false claim");
    assert!(
        matches!(&error, WorkflowV2AgentError::DeclaredArtifactAbsent(paths)
            if paths.len() == 1 && paths[0].starts_with(claimed)),
        "{error:?}"
    );
}

/// The host namespace is stripped even on an honest stop: an echoed gap for a
/// path the agent never declared does not survive as if the host had found it.
#[test]
fn an_echoed_host_gap_for_an_undeclared_path_is_dropped() {
    let (request, artifact_path) = request_with_required_artifact("impl-artifact-echo");
    write_project_artifact_file(&request, &artifact_path);
    let mut result = WorkflowV2Result::accepted("could not finish");
    result.status = WorkflowV2Status::Blocked;
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: "upstream-unavailable".to_string(),
        description: "the input was not available".to_string(),
        severity: Some("blocking".to_string()),
    });
    result.residual_gaps.push(crate::WorkflowV2ResidualGap {
        id: host_missing_artifact_gap_id("artifacts/echoed.json"),
        description:
            "missing project artifact evidence at artifacts/echoed.json: it does not exist"
                .to_string(),
        severity: Some("blocking".to_string()),
    });
    let output = serde_json::to_string(&result).expect("result json");

    let parsed = WorkflowV2AgentAdapter::new()
        .parse_agent_output(&request, &output)
        .expect("an honest stop is not re-asked");
    assert_eq!(parsed.status, WorkflowV2Status::Blocked);
    assert!(
        !parsed
            .residual_gaps
            .iter()
            .any(|gap| gap.id.starts_with("missing_project_artifact_")),
        "{:?}",
        parsed.residual_gaps
    );
}

/// The id the host mints for a missing artifact at `path` (mirrors
/// `project_artifacts::artifact_id_for_path`), so a pre-seeded gap collides
/// with the host's own and would dedup it if the namespace were not stripped.
fn host_missing_artifact_gap_id(path: &str) -> String {
    let id = path
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    format!("missing_project_artifact_{}", id.trim_matches('-'))
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
        target_ownership_scopes: Vec::new(),
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
