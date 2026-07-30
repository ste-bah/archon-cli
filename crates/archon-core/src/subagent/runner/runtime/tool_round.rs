use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::join_all;

use super::*;

struct PreparedTool {
    id: String,
    name: String,
    input: serde_json::Value,
    parse_error: Option<String>,
}

pub(super) async fn replay_tool_round(
    runner: &SubagentRunner,
    messages: &mut Vec<serde_json::Value>,
    text_content: String,
    thinking_blocks: BTreeMap<u32, PendingThinkingBlock>,
    pending_tools: Vec<PendingTool>,
    round_cancel: tokio_util::sync::CancellationToken,
) {
    record_assistant_tool_use_message(
        runner,
        messages,
        text_content,
        thinking_blocks,
        &pending_tools,
    );
    let prepared = prepare_tools_for_execution(runner, &pending_tools);
    let exec_results = execute_prepared_tools(runner, &prepared, round_cancel).await;
    record_tool_results(runner, messages, &prepared, exec_results);
    drain_pending_user_turns(runner, messages).await;
}

fn record_assistant_tool_use_message(
    runner: &SubagentRunner,
    messages: &mut Vec<serde_json::Value>,
    text_content: String,
    thinking_blocks: BTreeMap<u32, PendingThinkingBlock>,
    pending_tools: &[PendingTool],
) {
    let mut assistant_content: Vec<serde_json::Value> = Vec::new();
    if should_replay_signed_thinking(runner) {
        for block in thinking_blocks.values() {
            if !block.thinking.is_empty() {
                assistant_content.push(serde_json::json!({
                    "type": "thinking",
                    "thinking": block.thinking,
                    "signature": block.signature,
                }));
            }
        }
    }
    if !text_content.is_empty() {
        assistant_content.push(serde_json::json!({
            "type": "text",
            "text": text_content,
        }));
    }
    for tool in pending_tools {
        assistant_content.push(serde_json::json!({
            "type": "tool_use",
            "id": tool.id,
            "name": tool.name,
            "input": parse_tool_input_for_replay(runner, tool),
        }));
    }
    let assistant_msg = serde_json::json!({
        "role": "assistant",
        "content": assistant_content,
    });
    runner.record_transcript(&assistant_msg);
    messages.push(assistant_msg);
}

fn should_replay_signed_thinking(runner: &SubagentRunner) -> bool {
    matches!(
        runner.provider.compaction_policy().wire_shape,
        archon_llm::compaction_policy::WireShape::AnthropicMessages
            | archon_llm::compaction_policy::WireShape::VertexAnthropic
    )
}

fn parse_tool_input_for_replay(runner: &SubagentRunner, tool: &PendingTool) -> serde_json::Value {
    match crate::agent::tool_input_json::parse_pending_tool_input(
        &tool.name,
        &tool.id,
        &tool.input_json,
        tool_allows_empty_input(runner, &tool.name),
    ) {
        Ok(input) => input,
        Err(err) => {
            tracing::warn!(
                tool = %tool.name,
                tool_use_id = %tool.id,
                input_len = tool.input_json.len(),
                scope = "subagent",
                "{err}"
            );
            serde_json::json!({
                "_archon_malformed_tool_input": true,
                "error": err,
            })
        }
    }
}

fn prepare_tools_for_execution(
    runner: &SubagentRunner,
    pending_tools: &[PendingTool],
) -> Vec<PreparedTool> {
    let mut prepared = Vec::with_capacity(pending_tools.len());
    for tool in pending_tools {
        let (input, parse_error) = match crate::agent::tool_input_json::parse_pending_tool_input(
            &tool.name,
            &tool.id,
            &tool.input_json,
            tool_allows_empty_input(runner, &tool.name),
        ) {
            Ok(input) => (input, None),
            Err(err) => {
                tracing::warn!(
                    tool = %tool.name,
                    tool_use_id = %tool.id,
                    input_len = tool.input_json.len(),
                    scope = "subagent",
                    "{err}"
                );
                (serde_json::json!({}), Some(err))
            }
        };
        prepared.push(PreparedTool {
            id: tool.id.clone(),
            name: tool.name.clone(),
            input,
            parse_error,
        });
    }
    prepared
}

fn tool_allows_empty_input(runner: &SubagentRunner, name: &str) -> bool {
    runner
        .registry
        .lookup(name)
        .map(|tool_arc| {
            crate::agent::tool_input_json::schema_allows_empty_input(&tool_arc.input_schema())
        })
        .unwrap_or(false)
}

async fn execute_prepared_tools(
    runner: &SubagentRunner,
    prepared: &[PreparedTool],
    round_cancel: tokio_util::sync::CancellationToken,
) -> Vec<ToolResult> {
    let registry = Arc::clone(&runner.registry);
    let exec_futures: Vec<_> = prepared
        .iter()
        .map(|p| {
            let name = p.name.clone();
            let tool_use_id = p.id.clone();
            let input = p.input.clone();
            let parse_error = p.parse_error.clone();
            let registry = Arc::clone(&registry);
            let mut ctx = runner.tool_context.with_tool_run_attempt(tool_use_id, 0);
            ctx.cancel_parent = Some(round_cancel.child_token());
            async move {
                if let Some(err) = parse_error {
                    return ToolResult::error(err);
                }
                registry.dispatch(&name, input, &ctx).await
            }
        })
        .collect();
    join_all(exec_futures).await
}

fn record_tool_results(
    runner: &SubagentRunner,
    messages: &mut Vec<serde_json::Value>,
    prepared: &[PreparedTool],
    exec_results: Vec<ToolResult>,
) {
    for prepared_tool in prepared {
        record_tool_progress(runner, prepared_tool);
    }
    for (prepared_tool, result) in prepared.iter().zip(&exec_results) {
        runner.emit_activity_stream(
            "tool_result",
            summarize_tool_output(&result.content),
            Some(&prepared_tool.name),
            result.is_error,
        );
    }
    let (raw_transcript, canonical) = build_tool_result_messages(
        prepared,
        exec_results,
        crate::agent::tool_result_context::resolved_max_tool_result_bytes(
            runner.agent_config.context.max_tool_result_bytes,
            runner.provider.as_ref(),
        ),
    );
    runner.record_transcript(&raw_transcript);
    messages.push(canonical);
}

fn build_tool_result_messages(
    prepared: &[PreparedTool],
    exec_results: Vec<ToolResult>,
    max_tool_result_bytes: usize,
) -> (serde_json::Value, serde_json::Value) {
    let mut raw_results = Vec::with_capacity(prepared.len());
    let mut canonical_results = Vec::with_capacity(prepared.len());
    for (prepared_tool, result) in prepared.iter().zip(exec_results) {
        let raw = serde_json::json!({
            "type": "tool_result",
            "tool_use_id": prepared_tool.id,
            "content": result.content,
            "is_error": result.is_error,
        });
        let capped = crate::agent::tool_result_context::cap_tool_output_to_bytes(
            raw["content"].as_str().unwrap_or_default(),
            max_tool_result_bytes,
        );
        if capped.truncated {
            tracing::warn!(
                tool = %prepared_tool.name,
                tool_use_id = %prepared_tool.id,
                original_bytes = capped.original_bytes,
                stored_bytes = capped.stored_bytes,
                limit_bytes = capped.limit_bytes,
                "subagent tool result exceeded provider field policy"
            );
        }
        let mut canonical = raw.clone();
        canonical["content"] = serde_json::Value::String(capped.content);
        raw_results.push(raw);
        canonical_results.push(canonical);
    }
    (
        serde_json::json!({"role": "user", "content": raw_results}),
        serde_json::json!({"role": "user", "content": canonical_results}),
    )
}

fn record_tool_progress(runner: &SubagentRunner, prepared_tool: &PreparedTool) {
    if let Some(ref tracker) = runner.progress
        && let Ok(mut guard) = tracker.lock()
    {
        guard.tool_use_count += 1;
        if guard.recent_activities.len() >= 5 {
            guard.recent_activities.pop_front();
        }
        guard
            .recent_activities
            .push_back(crate::subagent::ToolActivity {
                tool_name: prepared_tool.name.clone(),
                timestamp: chrono::Utc::now(),
            });
        guard.last_update = chrono::Utc::now();
    }
}

async fn drain_pending_user_turns(runner: &SubagentRunner, messages: &mut Vec<serde_json::Value>) {
    let pending = runner.drain_pending_as_user_turns().await;
    for msg in pending {
        runner.record_transcript(&msg);
        messages.push(msg);
    }
}

#[cfg(test)]
mod result_boundary_tests {
    use super::*;

    fn prepared_tool() -> PreparedTool {
        PreparedTool {
            id: "tool-1".into(),
            name: "Read".into(),
            input: serde_json::json!({}),
            parse_error: None,
        }
    }

    #[test]
    fn raw_transcript_keeps_full_result_while_canonical_message_is_byte_bounded() {
        let raw_content = format!("HEAD{}TAIL", "é".repeat(100_000));
        let (raw_transcript, canonical) = build_tool_result_messages(
            &[prepared_tool()],
            vec![ToolResult::success(raw_content.clone())],
            4_096,
        );

        assert_eq!(
            raw_transcript["content"][0]["content"],
            serde_json::Value::String(raw_content)
        );
        let canonical_content = canonical["content"][0]["content"]
            .as_str()
            .expect("canonical tool result content");
        assert!(canonical_content.contains("omitted"));
        assert!(
            serde_json::to_vec(&serde_json::Value::String(canonical_content.into()))
                .expect("serialize canonical provider field")
                .len()
                <= 4_096
        );
    }

    #[test]
    fn raw_transcript_reopens_with_full_result_while_canonical_stays_capped() {
        let raw_content = format!("HEAD{}TAIL", "é".repeat(100_000));
        let (raw_transcript, canonical) = build_tool_result_messages(
            &[prepared_tool()],
            vec![ToolResult::success(raw_content.clone())],
            4_096,
        );
        let temp = tempfile::tempdir().expect("create transcript directory");
        let store = crate::agents::transcript::AgentTranscriptStore::with_base_dir(
            temp.path().to_path_buf(),
        );

        store.record_message("agent-1", &raw_transcript);
        let reopened = crate::agents::transcript::AgentTranscriptStore::with_base_dir(
            temp.path().to_path_buf(),
        );
        let persisted = reopened
            .get_transcript("agent-1")
            .expect("reload raw transcript");

        assert_eq!(persisted[0]["content"][0]["content"], raw_content);
        assert_ne!(canonical["content"][0]["content"], raw_content);
        assert!(
            canonical["content"][0]["content"]
                .as_str()
                .expect("canonical content")
                .contains("omitted")
        );
    }

    #[test]
    fn ordinary_tool_result_is_identical_in_raw_and_canonical_messages() {
        let (raw_transcript, canonical) = build_tool_result_messages(
            &[prepared_tool()],
            vec![ToolResult::success("small output")],
            4_096,
        );

        assert_eq!(raw_transcript, canonical);
    }
}
