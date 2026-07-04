use archon_workflow::{ProviderTier, StageKind, StageRunRequest};
use serde_json::json;

use super::workflow_live_runner::{activity_detail, allowed_tools, write_coordination_enabled};

fn request(input: serde_json::Value) -> StageRunRequest {
    request_with_task(input, "Implement")
}

fn request_with_task(input: serde_json::Value, task: &str) -> StageRunRequest {
    StageRunRequest {
        run_id: "wf-test".into(),
        stage_id: "stage".into(),
        stage_kind: StageKind::Implementation,
        agent: None,
        task: task.into(),
        attempt: 1,
        provider_tier: ProviderTier::Coder,
        depends_on: Vec::new(),
        input,
    }
}

#[test]
fn write_coordination_object_enables_boundary_mode() {
    let req = request(json!({"write_coordination": {"enabled": true}}));
    assert!(write_coordination_enabled(&req));
    assert!(!allowed_tools(&req).contains(&"Bash".to_string()));
}

#[test]
fn write_coordination_bool_enables_boundary_mode() {
    let req = request(json!({"write_coordination": true}));
    assert!(write_coordination_enabled(&req));
    assert!(!allowed_tools(&req).contains(&"Bash".to_string()));
}

#[test]
fn coordinated_implementation_with_verification_gets_bash() {
    let req = request_with_task(
        json!({"write_coordination": {"enabled": true}}),
        "Implement missing work and run focused tests.",
    );
    assert!(write_coordination_enabled(&req));
    assert!(allowed_tools(&req).contains(&"Bash".to_string()));
}

#[test]
fn activity_detail_names_stage_cwd_and_tool_mode() {
    let req = request(json!({"target_repository_root": "/tmp/project"}));
    let detail = activity_detail(&req, "stage running");

    assert!(detail.contains("stage=stage"));
    assert!(detail.contains("cwd=/tmp/project"));
    assert!(detail.contains("tool_mode=full"));
}

#[test]
fn generated_v2_read_only_verification_branch_stays_read_only() {
    let req = StageRunRequest {
        run_id: "wf-test".into(),
        stage_id: "readonly-discovery-verification-inventory".into(),
        stage_kind: StageKind::Agent,
        agent: Some("researcher".into()),
        task: "Inspect focused test commands and verification setup; do not run tests.".into(),
        attempt: 1,
        provider_tier: ProviderTier::Researcher,
        depends_on: Vec::new(),
        input: json!({
            "target_repository_root": "/tmp/project",
            "v2_call": {
                "id": "readonly-discovery-verification-inventory",
                "method": "agent",
                "role": "researcher",
                "write_mode": null,
                "target_files": []
            }
        }),
    };

    let tools = allowed_tools(&req);
    assert!(tools.contains(&"Read".to_string()));
    assert!(tools.contains(&"Grep".to_string()));
    assert!(!tools.contains(&"Bash".to_string()));
    assert!(!tools.contains(&"Write".to_string()));
    assert!(activity_detail(&req, "stage running").contains("tool_mode=read_only"));
}
