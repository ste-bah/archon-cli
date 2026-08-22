// The repeat-tool advisory reaching the model (#200 Phase 2).
//
// These drive the parent agent's real tool loop, `handle_pending_tool_round`,
// so they pin the *delivery* the dispatch-level tests deliberately do not:
// the reminder arrives as its own user turn after the round's tool results,
// and the `tool_result` recorded in history is byte for byte what the tool
// returned.

const REPEAT_TOOL_OUTPUT: &str = "no matches found";

struct RepeatFixedTool;

#[async_trait::async_trait]
impl archon_tools::tool::Tool for RepeatFixedTool {
    fn name(&self) -> &str {
        "Grep"
    }

    fn description(&self) -> &str {
        "repeat-tool reminder test"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }

    async fn execute(
        &self,
        _input: serde_json::Value,
        _ctx: &archon_tools::tool::ToolContext,
    ) -> archon_tools::tool::ToolResult {
        archon_tools::tool::ToolResult::success(REPEAT_TOOL_OUTPUT)
    }

    fn permission_level(
        &self,
        _input: &serde_json::Value,
    ) -> archon_tools::tool::PermissionLevel {
        archon_tools::tool::PermissionLevel::Safe
    }

    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }

    /// A search reads; it writes nothing. Declared so preflight does not take a
    /// working-tree baseline for a tool that cannot change one.
    fn working_tree_effect(&self) -> archon_tools::tool::WorkingTreeEffect {
        archon_tools::tool::WorkingTreeEffect::None
    }
}

fn repeat_tool_agent(session_id: &str) -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(RepeatFixedTool));
    Agent::new(
        Arc::new(MockLlmProvider),
        registry,
        AgentConfig {
            session_id: session_id.to_string(),
            ..AgentConfig::default()
        },
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    )
}

/// Run one tool round holding a single `Grep` call, the way the stream loop
/// does once the provider has finished emitting the block.
async fn repeat_tool_round(agent: &mut Agent, call_index: usize) {
    let pending = vec![PendingToolCall {
        id: format!("tool-{call_index}"),
        name: "Grep".to_string(),
        input_json: "{\"pattern\":\"needle\"}".to_string(),
    }];
    agent.state.add_assistant_message(vec![serde_json::json!({
        "type": "tool_use",
        "id": format!("tool-{call_index}"),
        "name": "Grep",
        "input": {"pattern": "needle"},
    })]);
    let mut iterations = 0;
    agent
        .handle_pending_tool_round(&pending, "mock", &mut iterations)
        .await;
}

fn repeat_tool_user_turns(agent: &Agent) -> Vec<String> {
    agent
        .state
        .messages
        .iter()
        .filter(|message| message["role"] == "user")
        .filter_map(|message| message["content"].as_str().map(str::to_string))
        .collect()
}

fn repeat_tool_results(agent: &Agent) -> Vec<serde_json::Value> {
    agent
        .state
        .messages
        .iter()
        .filter_map(|message| message["content"].as_array())
        .flatten()
        .filter(|block| block["type"] == "tool_result")
        .cloned()
        .collect()
}

#[tokio::test]
async fn main_repeat_tool_reminder_arrives_as_its_own_user_turn() {
    let mut agent = repeat_tool_agent("agent-loop-repeat-tool");

    for index in 0..2 {
        repeat_tool_round(&mut agent, index).await;
    }
    assert!(
        repeat_tool_user_turns(&agent)
            .iter()
            .all(|text| !text.contains("[repeat-tool guard]")),
        "two calls are not yet a run"
    );

    repeat_tool_round(&mut agent, 2).await;

    let reminders: Vec<String> = repeat_tool_user_turns(&agent)
        .into_iter()
        .filter(|text| text.contains("[repeat-tool guard]"))
        .collect();
    assert_eq!(reminders.len(), 1, "got {reminders:?}");
    assert!(reminders[0].contains("called Grep 3 times in a row"));
    assert_eq!(
        agent.state.messages.last().expect("a message")["role"],
        "user",
        "the reminder is appended after the round's tool results"
    );
}

/// The recorded `tool_result` is the tool's own output and nothing else. Fold
/// the reminder into it and the audit record shows a tool returning text it
/// never produced, charged against that result's byte budget.
#[tokio::test]
async fn main_repeat_tool_reminder_leaves_the_tool_result_byte_identical() {
    let mut agent = repeat_tool_agent("agent-loop-repeat-tool-bytes");

    for index in 0..3 {
        repeat_tool_round(&mut agent, index).await;
    }

    let results = repeat_tool_results(&agent);
    assert_eq!(results.len(), 3);
    for result in &results {
        assert_eq!(
            result["content"].as_str().expect("string content").as_bytes(),
            REPEAT_TOOL_OUTPUT.as_bytes(),
            "tool_result content must be exactly what the tool returned"
        );
    }
}

/// Turning the guard off leaves no guard, not a quieter one.
#[tokio::test]
async fn main_a_disabled_repeat_tool_guard_injects_nothing() {
    let (tx, _rx) = tokio::sync::mpsc::channel(AGENT_EVENT_CHANNEL_CAPACITY);
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(RepeatFixedTool));
    let mut agent = Agent::new(
        Arc::new(MockLlmProvider),
        registry,
        AgentConfig {
            session_id: "agent-loop-repeat-tool-disabled".to_string(),
            repeat_tool: crate::config::RepeatToolConfig {
                enabled: false,
                ..crate::config::RepeatToolConfig::default()
            },
            ..AgentConfig::default()
        },
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    );

    for index in 0..8 {
        repeat_tool_round(&mut agent, index).await;
    }

    assert!(
        repeat_tool_user_turns(&agent)
            .iter()
            .all(|text| !text.contains("[repeat-tool guard]"))
    );
}
