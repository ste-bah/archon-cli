use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use anyhow::Result;
use archon_pipeline::runner::{AgentExecutionRequest, LlmClient, LlmResponse};
use archon_tui::event_channel::bounded_tui_event_channel_with_capacity;
use archon_workflow::{ProviderTier, StageKind, StageRunRequest};

use super::workflow_live_runner::PipelineWorkflowRunner;

pub(crate) struct InvalidPlanner;

pub(crate) struct FlakyPlanner {
    pub(crate) calls: AtomicUsize,
    pub(crate) first_error: &'static str,
}

pub(crate) struct FlakyAgentClient {
    pub(crate) calls: AtomicUsize,
    pub(crate) first_error: &'static str,
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
    pub(crate) requests: Mutex<Vec<AgentExecutionRequest>>,
}

pub(crate) struct AlwaysInvalidItemsAgentClient {
    pub(crate) calls: AtomicUsize,
}

#[async_trait::async_trait]
impl LlmClient for InvalidPlanner {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        Ok(LlmResponse {
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
impl LlmClient for FlakyPlanner {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            anyhow::bail!(self.first_error);
        }
        Ok(LlmResponse {
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
impl LlmClient for GuttedImplementationPlanner {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
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
impl LlmClient for FlakyAgentClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        anyhow::bail!("test should use run_agent");
    }

    async fn run_agent(&self, _request: AgentExecutionRequest) -> Result<LlmResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            anyhow::bail!(self.first_error);
        }
        Ok(LlmResponse {
            content: "status: completed".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for SavedV2TemplateRunClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
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
impl LlmClient for GeneratedV2WorktreeRunClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        self.planner_calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
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

    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
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
                )?;
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
        Ok(LlmResponse {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for GeneratedV2RunClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let content = if call == 0 {
            r#"
export default async function workflow(w) {
  await w.agent("inspect", { role: "researcher", task: "Inspect the repository and summarize the current state." });
}
"#
            .to_string()
        } else {
            serde_json::json!({
                "status": "accepted",
                "summary": "Inspection completed with concrete evidence.",
                "evidence": [
                    {
                        "kind": "inspection",
                        "summary": "Read the repository entry points needed for the generated workflow."
                    }
                ],
                "artifacts": [],
                "commands_run": [],
                "files_read": [
                    {
                        "path": "Cargo.toml",
                        "purpose": "repository inspection"
                    }
                ],
                "files_changed": [],
                "task_coverage": [],
                "residual_gaps": []
            })
            .to_string()
        };
        Ok(LlmResponse {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for GeneratedV2FanoutRunClient {
    async fn send_message(
        &self,
        messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let content = match call {
            0 => {
                r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "researcher", task: "Return typed data.items for fanout." });
  const reviews = await w.fanout("review", inventory.items, { role: "critic", task: "Review one typed item.", maxParallelism: 3 });
  await w.reduce("final", {
    role: "reducer",
    inputs: [inventory, reviews],
    task: "Synthesize the resolved source_data."
  });
}
"#
                .to_string()
            }
            1 => serde_json::json!({
                "status": "accepted",
                "summary": "Inventory produced typed items.",
                "evidence": [
                    {"kind": "inspection", "summary": "Created typed item inventory for downstream fanout."}
                ],
                "artifacts": [],
                "commands_run": [],
                "files_read": [],
                "files_changed": [],
                "task_coverage": [],
                "residual_gaps": [],
                "data": {
                    "items": [
                        {"id": "a", "summary": "first"},
                        {"id": "b", "summary": "second"},
                        {"id": "c", "summary": "third"}
                    ]
                }
            })
            .to_string(),
            2..=4 => {
                let active = self.active_branches.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak_branches.fetch_max(active, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(25)).await;
                self.active_branches.fetch_sub(1, Ordering::SeqCst);
                serde_json::json!({
                    "status": "accepted",
                    "summary": "Branch reviewed typed item.",
                    "evidence": [
                        {"kind": "review", "summary": "Reviewed one fanout item."}
                    ],
                    "artifacts": [],
                    "commands_run": [],
                    "files_read": [],
                    "files_changed": [],
                    "task_coverage": [],
                    "residual_gaps": []
                })
                .to_string()
            }
            5 => {
                let prompt = messages
                    .first()
                    .and_then(|message| message.get("content"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if prompt.contains("source_data") && prompt.contains("review") {
                    self.reduce_source_seen.fetch_add(1, Ordering::SeqCst);
                }
                serde_json::json!({
                    "status": "accepted",
                    "summary": "Reducer synthesized typed source data.",
                    "evidence": [
                        {"kind": "review", "summary": "Reducer received typed fanout source data."}
                    ],
                    "artifacts": [],
                    "commands_run": [],
                    "files_read": [],
                    "files_changed": [],
                    "task_coverage": [],
                    "residual_gaps": []
                })
                .to_string()
            }
            _ => unreachable!("unexpected generated fanout test call"),
        };
        Ok(LlmResponse {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for GeneratedV2SlowFanoutRunClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let content = match call {
            0 => {
                r#"
export default async function workflow(w) {
  const inventory = await w.agent("inventory", { role: "planner", task: "Return typed slow review items." });
  const reviews = await w.fanout("review", inventory.items, { role: "critic", task: "Review one slow item.", maxParallelism: 2 });
  await w.reduce("final", { inputs: [reviews], task: "Summarize slow review evidence." });
}
"#
                .to_string()
            }
            1 => {
                let items = (0..20)
                    .map(|idx| serde_json::json!({"id": format!("item-{idx}"), "summary": "slow"}))
                    .collect::<Vec<_>>();
                serde_json::json!({
                    "status": "accepted",
                    "summary": "Inventory produced slow review items.",
                    "evidence": [
                        {"kind": "inspection", "summary": "Created typed slow fanout items."}
                    ],
                    "artifacts": [],
                    "commands_run": [],
                    "files_read": [],
                    "files_changed": [],
                    "task_coverage": [],
                    "residual_gaps": [],
                    "data": { "items": items }
                })
                .to_string()
            }
            _ => {
                self.launched_branches.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(100)).await;
                serde_json::json!({
                    "status": "accepted",
                    "summary": "Slow branch reviewed item.",
                    "evidence": [
                        {"kind": "review", "summary": "Reviewed one slow fanout item."}
                    ],
                    "artifacts": [],
                    "commands_run": [],
                    "files_read": [],
                    "files_changed": [],
                    "task_coverage": [],
                    "residual_gaps": []
                })
                .to_string()
            }
        };
        Ok(LlmResponse {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for InvalidItemsThenRepairAgentClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        anyhow::bail!("test should use run_agent");
    }

    async fn run_agent(&self, request: AgentExecutionRequest) -> Result<LlmResponse> {
        self.requests.lock().expect("requests lock").push(request);
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let content = if call == 0 {
            "Context was restored. What would you like to do next?".to_string()
        } else {
            r#"{"items":[{"id":"T001","task":"Implement T001","evidence":"source inspection found missing T001","target_files":["src/lib.rs"]}]}"#.to_string()
        };
        Ok(LlmResponse {
            content,
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
}

#[async_trait::async_trait]
impl LlmClient for AlwaysInvalidItemsAgentClient {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> Result<LlmResponse> {
        anyhow::bail!("test should use run_agent");
    }

    async fn run_agent(&self, _request: AgentExecutionRequest) -> Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            content: "Context was restored. What would you like to do next?".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
        })
    }
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

pub(crate) fn runner(llm: Arc<dyn LlmClient>) -> PipelineWorkflowRunner {
    let (tui_tx, _rx) = bounded_tui_event_channel_with_capacity(16);
    PipelineWorkflowRunner {
        llm,
        tui_tx,
        agent_names: Vec::new(),
        workspace_boundary_supported: false,
    }
}

pub(crate) fn boundary_runner(llm: Arc<dyn LlmClient>) -> PipelineWorkflowRunner {
    PipelineWorkflowRunner {
        workspace_boundary_supported: true,
        ..runner(llm)
    }
}
