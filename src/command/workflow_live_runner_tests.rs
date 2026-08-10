use std::sync::{Arc, Mutex};

use archon_workflow::{ProviderTier, StageKind, StageRunRequest, WorkflowStageRunner};
use serde_json::json;

use super::workflow_live_runner::allowed_tools;
use archon_workflow::stage_activity::activity_detail;
use archon_workflow::stage_command_policy::command_execution_stage;

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
fn coordinated_implementation_object_input_gets_bash() {
    // Every implementation branch now gets Bash so the coder can build/test its
    // edits, even under write-coordination and without the task naming tests.
    let req = request(json!({"write_coordination": {"enabled": true}}));
    assert!(allowed_tools(&req).contains(&"Bash".to_string()));
}

#[test]
fn coordinated_implementation_bool_input_gets_bash() {
    let req = request(json!({"write_coordination": true}));
    assert!(allowed_tools(&req).contains(&"Bash".to_string()));
}

#[test]
fn coordinated_implementation_with_verification_gets_bash() {
    let req = request_with_task(
        json!({"write_coordination": {"enabled": true}}),
        "Implement missing work and run focused tests.",
    );
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

#[test]
fn generated_v2_verification_wave_gets_command_execution() {
    let req = StageRunRequest {
        run_id: "wf-test".into(),
        stage_id: "verification-wave-task-verifier-2".into(),
        stage_kind: StageKind::Agent,
        agent: Some("coder".into()),
        task: "Run the declared focused verification commands.".into(),
        attempt: 1,
        provider_tier: ProviderTier::Coder,
        depends_on: Vec::new(),
        input: json!({
            "target_repository_root": "/tmp/project",
            "v2_call": {
                "id": "verification-wave-task-verifier-2",
                "method": "parallel",
                "role": "coder",
                "write_mode": null,
                "target_files": []
            }
        }),
    };

    assert!(command_execution_stage(&req));
    assert!(allowed_tools(&req).contains(&"Bash".to_string()));
}

// ---------------------------------------------------------------------------
// #161 — a stage branch on the task board
// ---------------------------------------------------------------------------

/// The board the whole test binary shares — never a second one. See
/// [`workflow_live_test_support::installed_board`].
fn resolved_board() -> Arc<archon_memory::MemoryGraph> {
    super::workflow_live_test_support::installed_board()
}

fn board_items(run_id: &str) -> Vec<archon_memory::board::BoardItem> {
    resolved_board()
        .list_board_items_by_run(run_id, &[])
        .expect("listing the run partition")
}

fn stage_request(run_id: &str) -> StageRunRequest {
    StageRunRequest {
        run_id: run_id.into(),
        stage_id: "implement".into(),
        task: "Implement the parser".into(),
        ..request(json!({}))
    }
}

/// A stage agent that reads the run's board from *inside* the running branch.
///
/// Asserting only on the closed item would not distinguish "raised at dispatch"
/// from "raised at the end", and the whole point of the change is that a run is
/// observable *while* it runs.
struct BoardProbeAgent {
    run_id: &'static str,
    /// `Err` is returned to the runner verbatim as a provider failure.
    reply: Result<&'static str, &'static str>,
    /// The run's partition as it looked mid-branch.
    seen: Mutex<Vec<archon_memory::board::BoardItem>>,
}

#[async_trait::async_trait]
impl archon_workflow::WorkflowLlmClient for BoardProbeAgent {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        unreachable!("a stage dispatches through run_agent")
    }

    async fn run_agent(
        &self,
        _request: archon_workflow::WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<archon_workflow::WorkflowAgentOutcome> {
        *self.seen.lock().expect("probe lock") = board_items(self.run_id);
        match self.reply {
            Ok(content) => Ok(archon_workflow::WorkflowAgentOutcome {
                content: content.to_string(),
                tool_uses: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
            }),
            Err(error) => Err(archon_workflow::WorkflowError::port(error)),
        }
    }
}

async fn run_probed_stage(
    run_id: &'static str,
    reply: Result<&'static str, &'static str>,
) -> (Arc<BoardProbeAgent>, Vec<archon_memory::board::BoardItem>) {
    resolved_board();
    let client = Arc::new(BoardProbeAgent {
        run_id,
        reply,
        seen: Mutex::new(Vec::new()),
    });
    let (stage_runner, _tui_rx) = super::workflow_live_test_support::runner(client.clone());
    let result = stage_runner.run_stage(stage_request(run_id)).await;
    assert_eq!(
        result.is_ok(),
        reply.is_ok(),
        "fixture drove the wrong path"
    );
    let items = board_items(run_id);
    (client, items)
}

/// The whole point of #161: dispatching a stage puts a claimed item on the
/// *run's* partition, and the branch's outcome closes it.
#[tokio::test]
async fn a_running_stage_holds_a_claimed_item_on_its_run_partition() {
    let (client, closed) = run_probed_stage("wf-board-live", Ok("status: completed")).await;

    let live = client.seen.lock().expect("probe lock").clone();
    assert_eq!(live.len(), 1, "one item per stage branch: {live:?}");
    let live = &live[0];
    assert_eq!(
        live.run_id, "wf-board-live",
        "partitioned by run, not stage"
    );
    assert_eq!(live.status, archon_memory::board::BoardStatus::Claimed);
    assert_eq!(
        live.claimed_by.as_deref(),
        Some(live.id.as_str()),
        "the item is held by the agent it names"
    );
    assert!(
        live.id
            .starts_with("wf-board-live-stage-implement-attempt-1-")
    );
    assert_eq!(
        live.kind,
        archon_memory::board::BoardItemKind::Note,
        "a branch raised as an issue would block the run's own drain gate"
    );
    assert!(live.title.contains("Implement the parser"));
    assert!(
        live.evidence.contains("Implement the parser"),
        "the brief the branch was handed is recorded: {}",
        live.evidence
    );

    assert_eq!(closed.len(), 1);
    assert_eq!(
        closed[0].status,
        archon_memory::board::BoardStatus::InReview,
        "a branch returning content is not evidence the work exists"
    );
}

/// A failed branch escalates. `resolved` would hide a run that got nothing done.
#[tokio::test]
async fn a_failed_stage_escalates_its_board_item() {
    let (_client, closed) =
        run_probed_stage("wf-board-failed", Err("subagent failed: fixture failure")).await;

    assert_eq!(closed.len(), 1);
    assert_eq!(
        closed[0].status,
        archon_memory::board::BoardStatus::Escalated
    );
}

/// A cancelled branch gives the work back rather than reporting a failure
/// nobody caused.
#[tokio::test]
async fn a_cancelled_stage_returns_its_item_to_the_pool() {
    let (_client, closed) = run_probed_stage("wf-board-cancelled", Err("subagent cancelled")).await;

    assert_eq!(closed.len(), 1);
    assert_eq!(closed[0].status, archon_memory::board::BoardStatus::Open);
    assert_eq!(closed[0].claimed_by, None, "the holder is cleared");
}

/// Every exit that is not a `finish` still closes the item: a required activity
/// emit is a `?`, and a stage that unwound through one must not leave a claimed
/// row that reads as live work forever.
#[tokio::test]
async fn a_stage_that_unwinds_on_a_dead_ui_still_closes_its_item() {
    let run_id = "wf-board-dead-ui";
    resolved_board();
    // Held at the provider until the UI is gone, so the dispatch activity is
    // delivered (the item is raised) and the completion emit is the one that
    // fails — `run_stage` then unwinds through `?` without reaching a `finish`.
    let client = Arc::new(
        super::workflow_live_test_support::CompletionBlockedAgentClient {
            started: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        },
    );
    let (ui_sink, mut tui_rx) = crate::command::tui_workflow_ui_sink::bounded_workflow_ui_sink(4);
    let stage_runner = super::workflow_live_runner::PipelineWorkflowRunner {
        llm: client.clone(),
        ui_sink,
        agent_names: Vec::new(),
        workspace_boundary_supported: false,
    };
    let stage = tokio::spawn(async move { stage_runner.run_stage(stage_request(run_id)).await });

    client.started.notified().await;
    let _running = tui_rx.recv().await.expect("dispatch activity");
    drop(tui_rx);
    client.release.notify_one();

    stage
        .await
        .expect("stage join")
        .expect_err("a closed UI fails the stage");

    let closed = board_items(run_id);
    assert_eq!(closed.len(), 1, "the item was raised at dispatch");
    assert_eq!(
        closed[0].status,
        archon_memory::board::BoardStatus::Escalated,
        "an unwound branch closes rather than leaking a claimed row"
    );
}

fn mcp_project() -> tempfile::TempDir {
    let project = tempfile::tempdir().expect("temp project");
    std::fs::write(
        project.path().join(".mcp.json"),
        r#"{
          "mcpServers": {
            "tradingview": {
              "command": "node",
              "toolPolicy": {
                "trustServerHints": false,
                "toolPermissions": {
                  "data_get_ohlcv": "safe",
                  "pine_check": "safe",
                  "pine_compile": "risky",
                  "pine_smart_compile": "risky",
                  "alert_create": "dangerous"
                }
              }
            }
          }
        }"#,
    )
    .expect("write MCP config");
    project
}

fn mcp_request(project: &std::path::Path, item: serde_json::Value) -> StageRunRequest {
    request(json!({
        "project_artifact_root": project,
        "item": item
    }))
}

#[test]
fn d37_declared_provider_and_pine_tools_are_exposed() {
    let project = mcp_project();
    let provider = mcp_request(
        project.path(),
        json!({
            "canonical_task_ids": ["TASK-DEMO-017"],
            "required_tools": ["data_get_ohlcv"]
        }),
    );
    let pine = mcp_request(
        project.path(),
        json!({
            "canonical_task_ids": ["TASK-DEMO-023"],
            "required_tools": ["pine_compile", "pine_smart_compile"]
        }),
    );

    let provider_tools = allowed_tools(&provider);
    let pine_tools = allowed_tools(&pine);
    assert!(provider_tools.contains(&"mcp__tradingview__data_get_ohlcv".to_string()));
    assert!(!provider_tools.contains(&"mcp__tradingview__pine_compile".to_string()));
    assert!(pine_tools.contains(&"mcp__tradingview__pine_compile".to_string()));
    assert!(pine_tools.contains(&"mcp__tradingview__pine_smart_compile".to_string()));
    assert!(!pine_tools.contains(&"mcp__tradingview__data_get_ohlcv".to_string()));
}

#[test]
fn d37_declared_tools_are_honored_but_dangerous_policy_is_not() {
    let project = mcp_project();
    let req = mcp_request(
        project.path(),
        json!({
            "canonical_task_ids": ["TASK-DEMO-001"],
            "required_tools": ["pine_check", "alert_create"]
        }),
    );

    let tools = allowed_tools(&req);
    assert!(tools.contains(&"mcp__tradingview__pine_check".to_string()));
    assert!(!tools.contains(&"mcp__tradingview__alert_create".to_string()));
}
