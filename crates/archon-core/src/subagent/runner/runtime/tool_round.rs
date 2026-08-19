use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::join_all;

use super::*;

/// A tool call with its input decoded exactly once (#171 part 8).
///
/// The replay message and the execution both need the same decoded input, and
/// both used to run `parse_pending_tool_input` over the same `input_json` —
/// twice per tool, plus two registry lookups to rebuild the same input schema
/// for the empty-input rule. Holding the outcome as a `Result` keeps the two
/// consumers on one decision: the replay message renders the malformed-input
/// marker from the error, and execution returns that same error verbatim.
struct PreparedTool {
    id: String,
    name: String,
    input: Result<serde_json::Value, String>,
}

pub(super) async fn replay_tool_round(
    runner: &SubagentRunner,
    messages: &mut MessageHistory,
    text_content: String,
    thinking_blocks: BTreeMap<u32, PendingThinkingBlock>,
    pending_tools: Vec<PendingTool>,
    round_cancel: tokio_util::sync::CancellationToken,
) {
    let prepared = prepare_tools_for_execution(runner, &pending_tools);
    record_assistant_tool_use_message(runner, messages, text_content, thinking_blocks, &prepared);
    let exec_results = execute_prepared_tools(runner, &prepared, round_cancel).await;
    // Before the results are recorded, not after: recording is what writes the
    // text into the history and the transcript, and an unrouted SendMessage
    // writes its own request envelope there as though it had been delivered.
    let exec_results = route_send_message_results(runner, &prepared, exec_results).await;
    record_tool_results(runner, messages, &prepared, exec_results);
    drain_pending_user_turns(runner, messages).await;
}

/// Deliver any `SendMessage` this round produced.
///
/// Without this a subagent's `SendMessage` returned its own serialized request
/// as the tool result: the model read that as confirmation and carried on,
/// while nothing had been routed anywhere (#184 M1).
///
/// The subagent host cannot resume a stopped target — see
/// [`crate::message_router::RouterHost::resume_stopped_agent`] — so a message
/// to a stopped peer is reported unreachable rather than silently starting a
/// whole agent run inside this one's tool round.
async fn route_send_message_results(
    runner: &SubagentRunner,
    prepared: &[PreparedTool],
    results: Vec<archon_tools::tool::ToolResult>,
) -> Vec<archon_tools::tool::ToolResult> {
    let (Some(manager), Some(self_id)) = (runner.subagent_manager(), runner.runner_agent_id())
    else {
        // No manager or no identity means this runner is not registered with a
        // session — a test harness or a bare runner. Routing would have nowhere
        // to deliver, so leave the results alone.
        return results;
    };

    if !prepared.iter().any(|tool| tool.name == "SendMessage") {
        return results;
    }

    let ctx = crate::message_router::RouterContext::new(
        manager,
        crate::message_router::SenderIdentity::Subagent {
            id: self_id.to_string(),
            lead_id: Some(crate::message_router::LEAD_QUEUE_ID.to_string()),
        },
    );
    let host = SubagentRouterHost;

    let mut routed = Vec::with_capacity(results.len());
    for (tool, result) in prepared.iter().zip(results) {
        routed.push(
            crate::message_router::maybe_route_send_message(&ctx, &host, &tool.name, result).await,
        );
    }
    routed
}

/// The subagent's side of the router: delivery only, no resume.
struct SubagentRouterHost;

#[async_trait::async_trait]
impl crate::message_router::RouterHost for SubagentRouterHost {
    async fn on_delivered(&self, target_id: &str, _message: &str) {
        tracing::debug!(target_id, "subagent routed a message");
    }
}

fn record_assistant_tool_use_message(
    runner: &SubagentRunner,
    messages: &mut MessageHistory,
    text_content: String,
    thinking_blocks: BTreeMap<u32, PendingThinkingBlock>,
    prepared: &[PreparedTool],
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
    for tool in prepared {
        assistant_content.push(serde_json::json!({
            "type": "tool_use",
            "id": tool.id,
            "name": tool.name,
            "input": replay_tool_input(tool),
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

/// The `input` field the assistant replay message carries for one tool.
///
/// A tool whose input never parsed is replayed as the malformed-input marker,
/// byte for byte what the two-parse shape produced.
fn replay_tool_input(tool: &PreparedTool) -> serde_json::Value {
    match &tool.input {
        Ok(input) => input.clone(),
        Err(err) => serde_json::json!({
            "_archon_malformed_tool_input": true,
            "error": err,
        }),
    }
}

fn prepare_tools_for_execution(
    runner: &SubagentRunner,
    pending_tools: &[PendingTool],
) -> Vec<PreparedTool> {
    let mut prepared = Vec::with_capacity(pending_tools.len());
    for tool in pending_tools {
        let input = crate::agent::tool_input_json::parse_pending_tool_input(
            &tool.name,
            &tool.id,
            &tool.input_json,
            tool_allows_empty_input(runner, &tool.name),
        );
        if let Err(ref err) = input {
            tracing::warn!(
                tool = %tool.name,
                tool_use_id = %tool.id,
                input_len = tool.input_json.len(),
                scope = "subagent",
                "{err}"
            );
        }
        prepared.push(PreparedTool {
            id: tool.id.clone(),
            name: tool.name.clone(),
            input,
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
            let registry = Arc::clone(&registry);
            let mut ctx = runner.tool_context.with_tool_run_attempt(tool_use_id, 0);
            ctx.cancel_parent = Some(round_cancel.child_token());
            // #193 Phase A. The parent agent's loop is not the only one that
            // runs tools; a policy only it consulted would leave a hole exactly
            // where Archon runs the most agents in parallel.
            let filesystem = runner.agent_config.filesystem;
            async move {
                let input = match input {
                    Ok(input) => input,
                    Err(err) => return ToolResult::error(err),
                };
                let observer = crate::agent::tool_preflight_freshness::observer_for(&ctx);
                if let Some(reason) = crate::agent::tool_preflight_freshness::refusal_for(
                    filesystem, &observer, &name, &input,
                ) {
                    return ToolResult::error(reason);
                }
                let result = registry.dispatch(&name, input.clone(), &ctx).await;
                crate::agent::tool_preflight_freshness::record(
                    filesystem,
                    &observer,
                    &name,
                    &input,
                    !result.is_error,
                );
                result
            }
        })
        .collect();
    join_all(exec_futures).await
}

fn record_tool_results(
    runner: &SubagentRunner,
    messages: &mut MessageHistory,
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

async fn drain_pending_user_turns(runner: &SubagentRunner, messages: &mut MessageHistory) {
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
            input: Ok(serde_json::json!({})),
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

    /// #171 part 8: one parse per tool must not change what a tool whose
    /// input never parsed replays, or what it hands the executor.
    #[test]
    fn malformed_tool_input_replays_the_marker_and_fails_execution_with_the_same_error() {
        let malformed = PreparedTool {
            id: "tool-9".into(),
            name: "Read".into(),
            input: Err("tool input was not valid JSON".into()),
        };

        assert_eq!(
            replay_tool_input(&malformed),
            serde_json::json!({
                "_archon_malformed_tool_input": true,
                "error": "tool input was not valid JSON",
            })
        );
        assert_eq!(
            malformed.input.unwrap_err(),
            "tool input was not valid JSON",
            "execution surfaces the parse error verbatim"
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
