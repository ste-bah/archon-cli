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
