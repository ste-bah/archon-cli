use super::*;

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
            stop_reason: None,
        })
    }

    async fn run_agent(
        &self,
        request: WorkflowAgentCall,
    ) -> archon_workflow::WorkflowResult<WorkflowAgentOutcome> {
        if let Some(outcome) = crate::command::workflow_live::audit_test_support::outcome(&request)
        {
            return Ok(outcome);
        }
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
            stop_reason: None,
        })
    }
}
