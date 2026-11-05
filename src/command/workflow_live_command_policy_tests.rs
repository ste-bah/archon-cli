use super::super::workflow_live_runner::full_tool_access;
use super::*;
use archon_workflow::stage_prompt::workflow_prompt;

#[test]
fn legacy_prose_cannot_change_tools_prompt_or_activity_mode() {
    for text in [
        "cargo test",
        "npm test",
        "pytest",
        "verification",
        "run focused tests",
    ] {
        let req = StageRunRequest {
            stage_id: "wave_post_tests".into(),
            stage_kind: StageKind::Agent,
            task: text.into(),
            ..request(json!({
                "stage_task": text,
                "dependencies": [{"artifact": text}],
                "stage_input": {"task": text},
                "fanout_item": {"task": text}
            }))
        };
        assert!(!command_execution_stage(&req));
        assert!(!allowed_tools(&req).contains(&"Bash".to_string()));
        assert!(!full_tool_access(&req));
        assert!(activity_detail(&req, "running").contains("tool_mode=read_only"));
        assert!(!workflow_prompt(&req).contains("Do not set a Bash `timeout` field"));
    }
}

#[test]
fn declared_legacy_shell_has_consistent_tools_prompt_and_activity() {
    for extra in [json!({"commands": true}), json!({"tools": ["Bash"]})] {
        let req = StageRunRequest {
            stage_kind: StageKind::Agent,
            task: "Inspect".into(),
            ..request(json!({"stage_extra": extra}))
        };
        assert!(command_execution_stage(&req));
        assert!(allowed_tools(&req).contains(&"Bash".to_string()));
        assert!(full_tool_access(&req));
        assert!(activity_detail(&req, "running").contains("tool_mode=full"));
        assert!(workflow_prompt(&req).contains("Do not set a Bash `timeout` field"));
    }
}
