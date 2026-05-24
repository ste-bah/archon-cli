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
) {
    record_assistant_tool_use_message(
        runner,
        messages,
        text_content,
        thinking_blocks,
        &pending_tools,
    );
    let prepared = prepare_tools_for_execution(runner, &pending_tools);
    let exec_results = execute_prepared_tools(runner, &prepared).await;
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
) -> Vec<ToolResult> {
    let registry = Arc::clone(&runner.registry);
    let exec_futures: Vec<_> = prepared
        .iter()
        .map(|p| {
            let name = p.name.clone();
            let input = p.input.clone();
            let parse_error = p.parse_error.clone();
            let registry = Arc::clone(&registry);
            let ctx = runner.tool_context.clone();
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
    let mut tool_results: Vec<serde_json::Value> = Vec::with_capacity(prepared.len());
    for (prepared_tool, result) in prepared.iter().zip(exec_results.into_iter()) {
        record_tool_progress(runner, prepared_tool);
        let context_output = crate::agent::tool_result_context::cap_tool_output_for_context(
            &prepared_tool.name,
            &result.content,
        );
        if context_output.truncated {
            tracing::warn!(
                tool = %prepared_tool.name,
                tool_use_id = %prepared_tool.id,
                original_chars = context_output.original_chars,
                stored_chars = context_output.stored_chars,
                limit_chars = context_output.limit_chars,
                scope = "subagent",
                "subagent tool output trimmed before model replay"
            );
        }
        tool_results.push(serde_json::json!({
            "type": "tool_result",
            "tool_use_id": prepared_tool.id,
            "content": context_output.content,
            "is_error": result.is_error,
        }));
        runner.emit_activity_stream(
            "tool_result",
            summarize_tool_output(&result.content),
            Some(&prepared_tool.name),
            result.is_error,
        );
    }
    let tool_result_msg = serde_json::json!({
        "role": "user",
        "content": tool_results,
    });
    runner.record_transcript(&tool_result_msg);
    messages.push(tool_result_msg);
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
