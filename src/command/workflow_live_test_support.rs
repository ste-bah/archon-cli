use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use archon_tui::event_channel::TuiEventReceiver;

use crate::command::tui_workflow_ui_sink::bounded_workflow_ui_sink;
use archon_workflow::{
    ProviderTier, StageKind, StageRunRequest, WorkflowAgentCall, WorkflowAgentOutcome,
    WorkflowLlmClient,
};

use super::workflow_live_runner::PipelineWorkflowRunner;

pub(crate) struct InvalidPlanner;

pub(crate) struct FlakyPlanner {
    pub(crate) calls: AtomicUsize,
    pub(crate) first_error: &'static str,
}

pub(crate) struct PlannerRepairRetryClient {
    pub(crate) calls: AtomicUsize,
    pub(crate) repair_started: tokio::sync::Notify,
    pub(crate) release_repair: tokio::sync::Notify,
}

pub(crate) struct FlakyAgentClient {
    pub(crate) calls: AtomicUsize,
    pub(crate) first_error: &'static str,
}

pub(crate) struct CompletionBlockedAgentClient {
    pub(crate) started: tokio::sync::Notify,
    pub(crate) release: tokio::sync::Notify,
}

pub(crate) struct GeneratedV2RunClient {
    pub(crate) calls: AtomicUsize,
}

pub(crate) struct GeneratedV2FanoutRunClient {
    pub(crate) calls: AtomicUsize,
    pub(crate) active_branches: AtomicUsize,
    pub(crate) peak_branches: AtomicUsize,
    pub(crate) reduce_source_seen: AtomicUsize,
}

pub(crate) struct GeneratedV2SlowFanoutRunClient {
    pub(crate) calls: AtomicUsize,
    pub(crate) launched_branches: AtomicUsize,
}

pub(crate) struct SavedV2TemplateRunClient {
    pub(crate) calls: AtomicUsize,
}

pub(crate) struct GeneratedV2WorktreeRunClient {
    pub(crate) planner_calls: AtomicUsize,
    pub(crate) agent_calls: AtomicUsize,
    pub(crate) implementation_cwd: Mutex<Option<PathBuf>>,
}

pub(crate) struct GuttedImplementationPlanner {
    pub(crate) calls: AtomicUsize,
}

pub(crate) struct InvalidItemsThenRepairAgentClient {
    pub(crate) calls: AtomicUsize,
    pub(crate) requests: Mutex<Vec<WorkflowAgentCall>>,
}

pub(crate) struct BlockedInvalidItemsAgentClient {
    pub(crate) calls: AtomicUsize,
    pub(crate) started: tokio::sync::Notify,
    pub(crate) release: tokio::sync::Notify,
}

pub(crate) struct AlwaysInvalidItemsAgentClient {
    pub(crate) calls: AtomicUsize,
}

#[async_trait::async_trait]
impl WorkflowLlmClient for InvalidPlanner {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        Ok(WorkflowAgentOutcome {
            content: r#"
export default async function workflow(w) {
  await w.agent("bad", { model: "claude-opus-4-8", task: "inspect" });
}
"#
            .to_string(),
            tool_uses: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
        })
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for FlakyPlanner {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            return Err(archon_workflow::WorkflowError::port(self.first_error));
        }
        Ok(WorkflowAgentOutcome {
            content: r#"
export default async function workflow(w) {
  await w.agent("discover", { role: "researcher", task: "inspect the repository" });
}
"#
            .to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for PlannerRepairRetryClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        match call {
            0 => Ok(WorkflowAgentOutcome {
                content: "export default async function workflow(w) { invalid(); }".to_string(),
                tool_uses: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
            }),
            1 => {
                self.repair_started.notify_one();
                self.release_repair.notified().await;
                Err(archon_workflow::WorkflowError::port(
                    "LLM stream error (server_error): temporary repair failure",
                ))
            }
            _ => Ok(WorkflowAgentOutcome {
                content: r#"export default async function workflow(w) {
  await w.agent("discover", { role: "researcher", task: "inspect" });
}"#
                .to_string(),
                tool_uses: Vec::new(),
                tokens_in: 1,
                tokens_out: 1,
            }),
        }
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for GuttedImplementationPlanner {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(WorkflowAgentOutcome {
            content: r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "Return one JSON object with data.items for implementation fanout." });
  const implemented = await w.fanout("implementationResults", inventory.items, { role: "coder", itemKind: "implementation", targetFilesFromItem: true, write: "coordinated", task: "Implement one inventory item." });
  const reviewed = await w.fanout("adversarialReview", implemented.items, { role: "critic", task: "Review one implementation result." });
  const report = await w.finalReport("finalReport", { inputs: [inventory, implemented, reviewed], task: "Produce final report from typed evidence." });
  await w.qualityGate("quality", { inputs: [report], task: "Accept only complete evidence." });
}
"#
            .to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for FlakyAgentClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        Err(archon_workflow::WorkflowError::port(
            "test should use run_agent",
        ))
    }

    async fn run_agent(
        &self,
        _request: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            return Err(archon_workflow::WorkflowError::port(self.first_error));
        }
        Ok(WorkflowAgentOutcome {
            content: "status: completed".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for CompletionBlockedAgentClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        unreachable!("test uses run_agent")
    }

    async fn run_agent(
        &self,
        _request: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.started.notify_one();
        self.release.notified().await;
        Ok(WorkflowAgentOutcome {
            content: "status: completed".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for SavedV2TemplateRunClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(WorkflowAgentOutcome {
            content: serde_json::json!({
                "status": "accepted",
                "summary": "Saved V2 command inspected concrete repository state.",
                "evidence": [
                    {
                        "kind": "inspection",
                        "summary": "Saved workflow command ran through the V2 agent path."
                    }
                ],
                "artifacts": [],
                "commands_run": [],
                "files_read": [
                    {
                        "path": "Cargo.toml",
                        "purpose": "saved V2 command smoke evidence"
                    }
                ],
                "files_changed": [],
                "task_coverage": [],
                "residual_gaps": []
            })
            .to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl WorkflowLlmClient for GeneratedV2WorktreeRunClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        self.planner_calls.fetch_add(1, Ordering::SeqCst);
        Ok(WorkflowAgentOutcome {
            content: r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "Return typed implementation inventory." });
  const implemented = await w.fanout("implementation", inventory.items, { role: "coder", itemKind: "implementation", targetFilesFromItem: true, write: "worktree", task: "Edit the assigned target file in the current repository root." });
  await w.finalReport("final", { inputs: [inventory, implemented], task: "Produce final report from typed evidence." });
}
"#
            .to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }

    async fn run_agent(
        &self,
        request: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        let call = self.agent_calls.fetch_add(1, Ordering::SeqCst);
        let content = match call {
            0 => serde_json::json!({
                "status": "accepted",
                "summary": "Inventory produced one worktree implementation item.",
                "evidence": [
                    {"kind": "inspection", "summary": "Created typed target ownership for worktree fanout."}
                ],
                "artifacts": [],
                "commands_run": [],
                "files_read": [],
                "files_changed": [],
                "task_coverage": [],
                "residual_gaps": [],
                "data": {
                    "items": [
                        {
                            "id": "T001",
                            "task": "Edit src/lib.rs",
                            "evidence": "src/lib.rs is the assigned target",
                            "target_files": ["src/lib.rs"]
                        }
                    ]
                }
            })
            .to_string(),
            1 => {
                let cwd = request.cwd.clone().expect("worktree cwd");
                std::fs::write(
                    cwd.join("src/lib.rs"),
                    "pub fn generated_worktree_value() -> usize { 1 }\n",
                )
                .map_err(archon_workflow::WorkflowError::port)?;
                *self.implementation_cwd.lock().expect("cwd lock") = Some(cwd);
                serde_json::json!({
                    "status": "accepted",
                    "summary": "Implementation edited src/lib.rs in isolated worktree.",
                    "evidence": [
                        {"kind": "implementation", "summary": "Edited the declared target file from the branch cwd."}
                    ],
                    "artifacts": [],
                    "commands_run": [
                        {
                            "kind": "test",
                            "command": "echo worktree implementation verification",
                            "status": "succeeded",
                            "exit_code": 0,
                            "output_summary": "worktree implementation verification"
                        }
                    ],
                    "files_read": [],
                    "files_changed": [
                        {"path": "src/lib.rs", "purpose": "declared target edit"}
                    ],
                    "task_coverage": [
                        {
                            "task_id": "T001",
                            "status": "accepted",
                            "summary": "src/lib.rs was changed in the isolated worktree and returned for canonical patch apply",
                            "evidence": [
                                {
                                    "kind": "implementation",
                                    "summary": "src/lib.rs changed in isolated worktree"
                                }
                            ]
                        }
                    ],
                    "residual_gaps": []
                })
                .to_string()
            }
            _ => unreachable!("unexpected worktree agent call"),
        };
        Ok(WorkflowAgentOutcome {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[path = "workflow_live_test_support_generated_clients.rs"]
mod workflow_live_test_support_generated_clients;

#[path = "workflow_live_test_support_invalid_items.rs"]
mod workflow_live_test_support_invalid_items;

/// The one task board this test binary installs.
///
/// `archon_tools::board::install_board_access` is a `OnceLock`: the first caller
/// wins and every later install is silently ignored. Two fixtures each building
/// their own graph therefore leaves one of them asserting against a board the
/// run never wrote to — and *which* one depends on test order, so the failure
/// appears only when the two happen to run in the same process. There is one
/// graph for the whole binary, and run partitioning is what keeps each fixture
/// from seeing the others' items, exactly as it keeps two concurrent workflow
/// runs apart in production.
///
/// Every full-lifecycle fixture installs it, not just the board tests: since
/// #142 the drain gate refuses a run it cannot check.
pub(crate) fn installed_board() -> Arc<archon_memory::MemoryGraph> {
    static BOARD: std::sync::OnceLock<Arc<archon_memory::MemoryGraph>> = std::sync::OnceLock::new();
    let graph = BOARD
        .get_or_init(|| Arc::new(archon_memory::MemoryGraph::in_memory().expect("board graph")));
    archon_tools::board::install_board_access(
        Arc::clone(graph) as Arc<dyn archon_memory::board::BoardAccess>
    );
    Arc::clone(graph)
}

pub(crate) fn request(input: serde_json::Value) -> StageRunRequest {
    StageRunRequest {
        run_id: "wf-test".into(),
        stage_id: "implement".into(),
        stage_kind: StageKind::Implementation,
        agent: None,
        task: "implement".into(),
        attempt: 1,
        provider_tier: ProviderTier::Coder,
        depends_on: Vec::new(),
        input,
    }
}

pub(crate) fn runner(
    llm: Arc<dyn WorkflowLlmClient>,
) -> (PipelineWorkflowRunner, TuiEventReceiver) {
    let (ui_sink, tui_rx) = bounded_workflow_ui_sink(16);
    (
        PipelineWorkflowRunner {
            llm,
            ui_sink,
            agent_names: Vec::new(),
            workspace_boundary_supported: false,
        },
        tui_rx,
    )
}

pub(crate) fn boundary_runner(
    llm: Arc<dyn WorkflowLlmClient>,
) -> (PipelineWorkflowRunner, TuiEventReceiver) {
    let (mut runner, tui_rx) = runner(llm);
    runner.workspace_boundary_supported = true;
    (runner, tui_rx)
}

/// A `TASK-*.md` file in the standard decomposed-PRD shape.
///
/// These fixtures used to carry bare `task_id:` / `depends_on:` lines with no
/// YAML block at all, which the old parser accepted by scanning raw text. That
/// partial parse is now a hard error, so every test fixture is written the way a
/// real task file is written: a fenced YAML block declaring every
/// contract-bearing key. `body` is appended verbatim for the markdown sections.
pub(crate) fn standard_task_file(
    task_id: &str,
    depends_on: &str,
    blocks: &str,
    body: &str,
) -> String {
    format!(
        "# {task_id}\n\n```yaml\ntask_id: {task_id}\ntitle: Fixture {task_id}\n\
         complexity: medium\nstatus: ready\ndepends_on: {depends_on}\nblocks: {blocks}\n\
         implements: []\nrequired_env_keys: []\nrequired_tools: []\n\
         deliverable_contracts: []\n```\n{body}"
    )
}
