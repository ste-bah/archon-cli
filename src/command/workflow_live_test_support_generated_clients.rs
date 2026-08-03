use super::*;

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
