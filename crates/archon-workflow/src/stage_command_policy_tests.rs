use super::*;
use crate::{ProviderTier, WorkflowSpec, WorkflowStore};
use serde_json::json;

fn request(input: serde_json::Value) -> StageRunRequest {
    StageRunRequest {
        run_id: "test".into(),
        stage_id: "inspect".into(),
        stage_kind: StageKind::Agent,
        agent: None,
        task: "Inspect evidence".into(),
        attempt: 1,
        provider_tier: ProviderTier::Researcher,
        depends_on: Vec::new(),
        input,
    }
}

#[test]
fn prose_and_legacy_ids_never_grant_commands() {
    for text in [
        "cargo test",
        "cargo check",
        "cargo build",
        "npm test",
        "pytest",
        "run focused tests",
        "verification",
        "quality gate",
        "lint",
    ] {
        for location in [
            "task",
            "stage_task",
            "dependency",
            "stage_input",
            "fanout_item",
        ] {
            let mut req = request(json!({}));
            if location == "task" {
                req.task = text.into();
            } else {
                req.input[location] = json!({"text": text});
                if location == "stage_task" {
                    req.input[location] = json!(text);
                }
            }
            assert!(!command_execution_stage(&req), "{location}: {text}");
            assert!(
                !crate::stage_prompt::workflow_prompt(&req)
                    .contains("Do not set a Bash `timeout` field")
            );
        }
    }
    for id in [
        "focused_tests",
        "wave_post_tests",
        "audit_verification",
        "verification-wave-1",
    ] {
        let mut req = request(json!({}));
        req.stage_id = id.into();
        assert!(!command_execution_stage(&req), "{id}");
    }
}

#[test]
fn only_declared_capabilities_grant_commands() {
    for extra in [
        json!({"commands": true}),
        json!({"tools": ["Bash"]}),
        json!({"allowed_tools": "shell"}),
        json!({"required_tools": ["Read", "bAsH"]}),
    ] {
        let req = request(json!({"stage_extra": extra}));
        assert!(command_execution_stage(&req));
        assert!(
            crate::stage_prompt::workflow_prompt(&req)
                .contains("Do not set a Bash `timeout` field")
        );
    }
    for input in [
        json!({}),
        json!({"commands": true}),
        json!({"tools": ["Bash"]}),
        json!({"stage_input": {"commands": true, "tools": ["Bash"]}}),
        json!({"stage_extra": {"commands": false}}),
        json!({"stage_extra": {"commands": "true", "tools": ["Read", "run Bash"]}}),
        json!({"v2_call": {"method": "unknown"}, "stage_task": "cargo test"}),
    ] {
        assert!(!command_execution_stage(&request(input)));
    }
    let mut req = request(json!({}));
    req.stage_kind = StageKind::Implementation;
    assert!(command_execution_stage(&req));
    assert!(command_execution_stage(&request(json!({
        "v2_call": {"id": "edit", "method": "implementation"}
    }))));
}

#[test]
fn typed_verification_grants_require_admitted_kind_or_host_plan() {
    for id in [
        "verification-wave-1",
        "review-verification-wave-2",
        "inspect",
    ] {
        let mut call = crate::WorkflowV2HostCall {
            id: id.into(),
            method: crate::WorkflowV2HostMethod::Agent,
            write_mode: None,
            options: Default::default(),
        };
        assert!(!v2_call_runs_commands(&call));
        assert!(!command_execution_stage(&request(json!({
            "v2_call": {"id": id, "method": "agent"}
        }))));
        call.options.item_kind = Some("focused_verification".into());
        assert!(v2_call_runs_commands(&call));
        call.options.item_kind = None;
        call.options.host_planned = true;
        assert_eq!(v2_call_runs_commands(&call), id != "inspect");
    }
    let mut granted = request(json!({"v2_call": {"method": "agent"}}));
    granted.input["stage_extra"] = json!({"tools": ["Bash"], "read_only_shell": true});
    assert!(command_execution_stage(&granted));
    assert!(read_only_shell_grant(&granted.input));
}

#[test]
fn yaml_stage_declarations_survive_stage_and_fanout_context() {
    for declaration in ["", "commands: true", "tools: [Bash]", "commands: false"] {
        let yaml = format!(
            "schema: archon.workflow.v1\nname: grants\ntask: inspect\nstages:\n  - id: inspect\n    kind: fanout\n    {declaration}\n    input:\n      items: [{{id: item, task: cargo test}}]\n"
        );
        let spec = WorkflowSpec::from_yaml(&yaml).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::new(temp.path());
        let run = store.create_run(spec).unwrap();
        let stage = &run.spec.stages[0];
        let expected = matches!(declaration, "commands: true" | "tools: [Bash]");
        let req = crate::request::stage_request(&store, &run, stage).unwrap();
        assert_eq!(command_execution_stage(&req), expected, "{declaration}");
        let item = &crate::fanout::extract_items(stage)[0];
        let req = crate::request::fanout_item_request(&store, &run, stage, item).unwrap();
        assert_eq!(command_execution_stage(&req), expected, "{declaration}");
    }
}

#[test]
fn commands_boolean_is_legacy_only_even_for_null_typed_marker() {
    for call in [json!(null), json!({"id": "inspect", "method": "agent"})] {
        let input = json!({"v2_call": call, "stage_extra": {"commands": true}});
        assert!(!command_execution_stage(&request(input.clone())));
    }
}

#[test]
fn typed_required_tools_grants_shell_after_real_context_projection() {
    for entry in ["bash", "BASH", "shell"] {
        let spec = WorkflowSpec::from_yaml(&format!("schema: archon.workflow.v1\nname: typed-tools\ntask: inspect\nstages:\n- id: inspect\n  kind: agent\n  required_tools: {entry}\n")).unwrap();
        assert_eq!(spec.stages[0].required_tools, [entry]);
        let dir = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(dir.path());
        let run = store.create_run(spec).unwrap();
        let input = crate::context::stage_input(&store, &run, &run.spec.stages[0]).unwrap();
        assert!(command_execution_stage(&request(input)));
    }
}
