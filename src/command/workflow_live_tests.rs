use std::path::PathBuf;
use std::sync::Arc;

use crate::command::tui_workflow_ui_sink::bounded_workflow_ui_sink;
use archon_workflow::{
    CommandAction, RunStatus, StageKind, StageRunRequest, StageStatus, WorkflowRun, WorkflowSpec,
    WriteBoundaryProbe,
};
use serde_json::json;

use super::workflow_live_prompt::{harness_planner_prompt, workflow_prompt};
use super::workflow_live_runner::{
    allowed_tools, request_target_repository_root, workflow_agent_ordinal,
    workflow_agent_session_id, workflow_stage_system_context,
};
use super::workflow_live_test_support::{InvalidPlanner, boundary_runner, request};
use super::{spawn_live_workflow, terminal_resume_message};

#[test]
fn workflow_and_session_paths_do_not_ignore_tui_delivery() {
    fn collect(path: &std::path::Path, offenders: &mut Vec<std::path::PathBuf>) {
        if path.is_file() {
            inspect_source(path, offenders);
            return;
        }
        for entry in std::fs::read_dir(path).expect("read source directory") {
            let path = entry.expect("read source entry").path();
            if path.is_dir() {
                collect(&path, offenders);
            } else {
                inspect_source(&path, offenders);
            }
        }
    }

    fn inspect_source(path: &std::path::Path, offenders: &mut Vec<std::path::PathBuf>) {
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") || name.contains("test") {
            return;
        }
        let compact: String = std::fs::read_to_string(path)
            .expect("read source")
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        // Two vocabularies, because two layers now emit. `session*` still
        // holds a `TuiEventSender` directly; workflow execution reaches the
        // same channel through `archon_workflow::ui_sink_port`. Dropping a
        // status is the same bug either way, so both spellings are offences.
        let ignores_tui_delivery = compact.split(';').any(|statement| {
            statement.contains("let_=")
                && (statement.contains(".send(TuiEvent")
                    || statement.contains(".send(archon_tui::app::TuiEvent")
                    || statement.contains(".emit(WorkflowUiEvent")
                    || statement.contains(".emit(archon_workflow::WorkflowUiEvent"))
        });
        if ignores_tui_delivery {
            offenders.push(path.to_path_buf());
        }
    }

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    collect(&root.join("session"), &mut offenders);
    collect(&root.join("session_loop"), &mut offenders);
    for entry in std::fs::read_dir(root.join("command")).expect("read command directory") {
        let path = entry.expect("read command entry").path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("workflow_live") && !name.contains("test"))
        {
            collect(&path, &mut offenders);
        }
    }

    assert!(
        offenders.is_empty(),
        "production paths ignore bounded TUI delivery: {offenders:?}"
    );
}

#[tokio::test]
async fn closed_tui_prevents_workflow_planner_launch() {
    let root = tempfile::tempdir().expect("tempdir");
    let planner = Arc::new(
        super::workflow_live_test_support::GuttedImplementationPlanner {
            calls: std::sync::atomic::AtomicUsize::new(0),
        },
    );
    let (ui_sink, rx) = bounded_workflow_ui_sink(1);
    drop(rx);

    spawn_live_workflow(
        root.path().to_path_buf(),
        CommandAction::Plan {
            task: "must not launch".into(),
        },
        planner.clone(),
        ui_sink,
        None,
    );
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        planner.calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "workflow planner launched after status delivery failed"
    );
}

#[test]
fn workflow_live_uses_target_repository_root_as_subagent_cwd() {
    let req = request(json!({
        "target_repository_root": "/tmp/target-repo",
    }));

    assert_eq!(
        request_target_repository_root(&req),
        Some(PathBuf::from("/tmp/target-repo"))
    );
}

#[test]
fn fanout_item_subagent_ordinals_are_unique_per_stage_item() {
    let first = StageRunRequest {
        stage_id: "adversarial_review_inventory-0".into(),
        ..request(json!({}))
    };
    let second = StageRunRequest {
        stage_id: "adversarial_review_inventory-1".into(),
        ..request(json!({}))
    };

    assert_ne!(
        workflow_agent_ordinal(&first),
        workflow_agent_ordinal(&second)
    );
}

#[test]
fn workflow_stage_session_ids_are_isolated_per_attempt() {
    let first = StageRunRequest {
        stage_id: "discover inventory".into(),
        attempt: 1,
        ..request(json!({}))
    };
    let second = StageRunRequest {
        stage_id: "discover inventory".into(),
        attempt: 2,
        ..request(json!({}))
    };

    assert_eq!(
        workflow_agent_session_id(&first),
        "wf-test-stage-discover-inventory-attempt-1"
    );
    assert_eq!(
        workflow_agent_session_id(&second),
        "wf-test-stage-discover-inventory-attempt-2"
    );
    assert_ne!(
        workflow_agent_session_id(&first),
        workflow_agent_session_id(&second)
    );
}

#[test]
fn workflow_stage_system_context_rejects_restored_context() {
    let req = StageRunRequest {
        stage_id: "discover".into(),
        attempt: 3,
        ..request(json!({}))
    };
    let system = workflow_stage_system_context(&req);

    assert!(system.contains("fresh workflow stage invocation"));
    assert!(system.contains("Ignore any restored conversational context"));
    assert!(system.contains("do not return restored-context summaries"));
    assert!(system.contains("attempt 3"));
}

#[test]
fn terminal_failed_live_resume_reports_restart_stage_command() {
    let spec = WorkflowSpec::from_yaml(
        r#"
schema: archon.workflow.v1
name: terminal-resume-guard
task: verify terminal resume guard
stages:
  - id: discover
    kind: agent
"#,
    )
    .expect("valid spec");
    let mut run = WorkflowRun::new(spec, "/tmp/workflows/wf-test");
    run.id = "wf-terminal".to_string();
    run.status = RunStatus::Failed;
    run.stages
        .get_mut("discover")
        .expect("discover stage")
        .status = StageStatus::Failed;

    let message = terminal_resume_message(&run).expect("terminal failed message");

    assert!(message.contains("cannot be resumed directly"));
    assert!(message.contains("/workflow repair wf-terminal"));
    assert!(message.contains("/workflow continue wf-terminal"));
    assert!(message.contains("/workflow restart task wf-terminal <task-id>"));
    assert!(message.contains("Debug detail: failed internal stage is discover"));
    assert!(!message.contains("/workflow restart-stage wf-terminal discover"));
}

#[test]
fn workflow_live_omits_empty_target_repository_root() {
    let req = request(json!({
        "target_repository_root": " ",
    }));

    assert_eq!(request_target_repository_root(&req), None);
}

#[test]
fn focused_test_workflow_stages_can_execute_commands_without_write_tools() {
    let req = StageRunRequest {
        stage_id: "focused_tests-8".into(),
        stage_kind: StageKind::Agent,
        task: "Run focused cargo test evidence for TASK-TRL-011".into(),
        ..request(json!({}))
    };
    let tools = allowed_tools(&req);

    assert!(tools.contains(&"Bash".to_string()));
    assert!(tools.contains(&"Read".to_string()));
    assert!(!tools.contains(&"Write".to_string()));
    assert!(!tools.contains(&"Edit".to_string()));
}

#[test]
fn coordinated_implementation_keeps_bash_for_workspace_verification() {
    let req = request(json!({
        "write_coordination": true,
        "target_repository_root": "/tmp/isolated-repo",
    }));
    let tools = allowed_tools(&req);

    assert!(tools.contains(&"Read".to_string()));
    assert!(tools.contains(&"Write".to_string()));
    assert!(tools.contains(&"ApplyPatch".to_string()));
    assert!(tools.contains(&"Bash".to_string()));
}

#[test]
fn serial_implementation_keeps_bash_available() {
    let req = request(json!({}));
    let tools = allowed_tools(&req);

    assert!(tools.contains(&"Bash".to_string()));
}

#[test]
fn workflow_live_reports_backing_workspace_boundary_support() {
    let (stage_runner, _tui_rx) = boundary_runner(Arc::new(InvalidPlanner));

    assert!(stage_runner.supports_workspace_boundary());
}

#[test]
fn post_remediation_test_stages_can_execute_commands_without_write_tools() {
    let req = StageRunRequest {
        stage_id: "wave2_post_tests".into(),
        stage_kind: StageKind::Agent,
        task: "Run focused post-remediation tests for T010/T020/T030 and capture exact commands/results.".into(),
        ..request(json!({}))
    };
    let tools = allowed_tools(&req);

    assert!(tools.contains(&"Bash".to_string()));
    assert!(tools.contains(&"Read".to_string()));
    assert!(!tools.contains(&"Write".to_string()));
    assert!(!tools.contains(&"Edit".to_string()));
}

#[test]
fn command_stage_prompt_uses_configured_bash_timeout() {
    let req = StageRunRequest {
        stage_id: "wave2_post_tests".into(),
        stage_kind: StageKind::Agent,
        task: "Run focused post-remediation tests for T010/T020/T030 and capture exact commands/results.".into(),
        ..request(json!({}))
    };
    let prompt = workflow_prompt(&req);

    assert!(prompt.contains("rely on the configured `tools.bash_timeout`"));
    assert!(prompt.contains("Do not set a Bash `timeout` field"));
    assert!(prompt.contains("do not wrap commands with shell-level `timeout`/`gtimeout`"));
    assert!(prompt.contains("Do not mark timed-out commands as completed or verified"));
}

#[test]
fn live_prompt_preserves_project_root_artifact_resolution() {
    let req = StageRunRequest {
        stage_id: "post-artifact-repair-review".into(),
        stage_kind: StageKind::Agent,
        task: "Adversarially review required artifact repairs.".into(),
        ..request(json!({}))
    };
    let prompt = workflow_prompt(&req);

    assert!(prompt.contains("Runtime evidence guardrails"));
    assert!(prompt.contains("`project_root`"));
    assert!(prompt.contains("Relative `.archon/...` deliverables are project-root artifacts"));
    assert!(prompt.contains("not under `target_repository_root`"));
}

#[test]
fn live_prompt_treats_empty_remediation_inventory_as_verified_noop() {
    let req = StageRunRequest {
        stage_id: "post-remediation-focused-tests".into(),
        stage_kind: StageKind::Agent,
        task: "Run focused tests required by the remediation inventory.".into(),
        ..request(json!({}))
    };
    let prompt = workflow_prompt(&req);

    assert!(prompt.contains("remediation inventory is exactly `{\"items\": []}`"));
    assert!(prompt.contains("return `status: verified` with no-op evidence"));
    assert!(prompt.contains("Do not return `status: unverifiable` only because"));
}

#[test]
fn live_prompt_requires_structured_items_for_item_producer_stage() {
    let req = StageRunRequest {
        stage_id: "discover".into(),
        stage_kind: StageKind::Agent,
        task: "Produce an implementation inventory.".into(),
        ..request(json!({
            "stage_extra": {
                "outputs": ["items"]
            }
        }))
    };
    let prompt = workflow_prompt(&req);

    assert!(prompt.contains("Structured item output contract"));
    assert!(prompt.contains("top-level `items` array"));
    assert!(prompt.contains("Do not return only markdown/prose"));
}

#[test]
fn live_prompt_does_not_add_item_contract_to_plain_stage() {
    let req = StageRunRequest {
        stage_id: "synthesize".into(),
        stage_kind: StageKind::Reduce,
        task: "Summarize findings.".into(),
        ..request(json!({}))
    };
    let prompt = workflow_prompt(&req);

    assert!(!prompt.contains("machine-readable fanout item producer"));
}

#[test]
fn command_stage_prompt_includes_platform_cargo_policy() {
    let req = StageRunRequest {
        stage_id: "wave5_tests".into(),
        stage_kind: StageKind::Agent,
        task: "Run focused tests for wave 5 and capture exact commands/results.".into(),
        ..request(json!({}))
    };
    let prompt = workflow_prompt(&req);

    assert!(prompt.contains("Cargo command policy for this host"));
    assert!(prompt.contains("Prefer exact package + test-target commands"));
    assert!(prompt.contains("reserve broad workspace checks for final quality gates"));
    assert!(prompt.contains("adapt the commands and report the adaptation"));
}

#[cfg(target_os = "macos")]
#[test]
fn command_stage_prompt_does_not_treat_wsl_jobs_as_macos_default() {
    let req = StageRunRequest {
        stage_id: "focused_tests".into(),
        stage_kind: StageKind::Agent,
        task: "Run focused cargo tests.".into(),
        ..request(json!({}))
    };
    let prompt = workflow_prompt(&req);

    assert!(prompt.contains("Cargo command policy for this host (macOS)"));
    assert!(prompt.contains("Native macOS: do not add `-j1` or `--jobs 1`"));
}

#[test]
fn harness_planner_prompt_requires_restricted_host_api() {
    let prompt = harness_planner_prompt("Implement a workflow task.", None);

    assert!(prompt.contains("export default async function workflow(w)"));
    assert!(prompt.contains("Use only these host API calls"));
    assert!(prompt.contains("w.agent"));
    assert!(prompt.contains("w.fanout"));
    assert!(prompt.contains("w.parallel"));
    assert!(prompt.contains("w.reduce"));
    assert!(prompt.contains("w.implementation"));
    assert!(prompt.contains("w.qualityGate"));
    assert!(prompt.contains("w.humanGate"));
    assert!(prompt.contains("w.checkpoint"));
    assert!(prompt.contains("w.saveArtifact"));
    assert!(prompt.contains("w.requireArtifact"));
    assert!(prompt.contains("w.finalReport"));
    assert!(prompt.contains("non-empty stable string id as its first argument"));
    assert!(prompt.contains("deterministic ids with a literal prefix"));
    assert!(prompt.contains("ordinary JavaScript control flow"));
    assert!(prompt.contains("non-accepted semantic statuses do not throw"));
    assert!(prompt.contains("Do not import modules"));
    assert!(prompt.contains("Return only JavaScript for workflow.js"));
}

#[test]
fn harness_planner_prompt_keeps_workflows_task_shaped() {
    let prompt = harness_planner_prompt(
        "Implement a Rust workflow task and run focused tests.",
        None,
    );

    assert!(prompt.contains("Shape the workflow to the task"));
    assert!(prompt.contains("Audit/review/research/planning is usually read-only"));
    assert!(prompt.contains("Small known edits use w.implementation with targetFiles"));
    assert!(prompt.contains("Broad migrations use inventory variables"));
    assert!(prompt.contains("Add remediation/fix calls when the task asks"));
    assert!(prompt.contains("fix every issue found before continuing"));
}

#[test]
fn harness_planner_prompt_requires_explicit_fanout_item_contracts() {
    let prompt = harness_planner_prompt("Implement a decomposed PRD.", None);

    assert!(prompt.contains("Every w.fanout or w.parallel that iterates work"));
    assert!(prompt.contains("w.fanout(\"id\", inventory.items"));
    assert!(prompt.contains("w.parallel(\"id\", items"));
    assert!(prompt.contains("actual typed JavaScript array as its second argument"));
    assert!(prompt.contains("top-level `items: [...]`"));
    assert!(prompt.contains("target_files"));
}

#[test]
fn harness_planner_prompt_separates_report_artifacts_from_repo_implementation() {
    let prompt = harness_planner_prompt(
        "Implement T140 readiness and adversarial review artifacts.",
        None,
    );

    assert!(prompt.contains("Report-only deliverables"));
    assert!(prompt.contains("w.agent or w.reduce artifacts"));
    assert!(prompt.contains("w.requireArtifact"));
    assert!(prompt.contains("do not model reports as implementation work"));
    assert!(prompt.contains("w.finalReport must include the evidence-producing"));
}

#[test]
fn generated_run_branch_is_isolated_from_legacy_executor_dispatch() {
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/command/workflow_live.rs"),
    )
    .expect("read workflow_live source");
    let run_branch = source
        .split("CommandAction::Run { task, decomposed } => {")
        .nth(1)
        .and_then(|rest| rest.split("CommandAction::RunSpec").next())
        .expect("generated run branch");

    assert!(run_branch.contains("run_generated_v2_workflow"));
    assert!(!run_branch.contains("start_with_harness"));
    assert!(!run_branch.contains("execute_with_runner"));
    assert!(!run_branch.contains("WorkflowExecutor::"));
}

#[test]
fn generated_live_path_has_no_yaml_repair_fallback() {
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/command/workflow_live.rs"),
    )
    .expect("read workflow_live source");

    assert!(!source.contains("validate_or_repair_plan"));
    assert!(!source.contains("request_repaired_plan"));
    assert!(!source.contains("generated YAML failed validation"));
}

#[test]
fn explicit_stage_extra_can_request_bash() {
    let req = StageRunRequest {
        stage_id: "validate".into(),
        task: "Validate generated outputs".into(),
        ..request(json!({
            "stage_extra": {
                "allowed_tools": ["Read", "Bash"]
            }
        }))
    };

    assert!(allowed_tools(&req).contains(&"Bash".to_string()));
}
