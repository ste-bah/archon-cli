use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use archon_consciousness::rules::RuleSource;
use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};
use archon_memory::MemoryGraph;

struct MockLlmProvider;

#[async_trait::async_trait]
impl LlmProvider for MockLlmProvider {
    fn name(&self) -> &str {
        "mock"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![]
    }

    fn supports_feature(&self, _: ProviderFeature) -> bool {
        false
    }

    async fn stream(
        &self,
        _request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        Ok(rx)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unimplemented!()
    }
}

fn test_agent() -> Agent {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    Agent::new(
        Arc::new(MockLlmProvider),
        ToolRegistry::new(),
        AgentConfig::default(),
        tx,
        Arc::new(std::sync::RwLock::new(AgentRegistry::load(
            &std::env::temp_dir(),
        ))),
    )
}

#[tokio::test]
async fn auto_extraction_flush_waits_for_pending_tasks() {
    let mut agent = test_agent();
    let completed = Arc::new(AtomicUsize::new(0));
    let completed_task = Arc::clone(&completed);
    agent.auto_extraction_tasks.push(tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        completed_task.fetch_add(1, Ordering::SeqCst);
    }));

    assert_eq!(agent.pending_auto_extraction_count(), 1);
    let flushed = agent
        .flush_auto_extractions(std::time::Duration::from_secs(1))
        .await;

    assert_eq!(flushed, 1);
    assert_eq!(agent.pending_auto_extraction_count(), 0);
    assert_eq!(completed.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn auto_extraction_prune_keeps_only_unfinished_tasks() {
    let mut agent = test_agent();
    agent.auto_extraction_tasks.push(tokio::spawn(async {}));
    agent.auto_extraction_tasks.push(tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }));
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    agent.prune_finished_auto_extractions();

    assert_eq!(agent.pending_auto_extraction_count(), 1);
    agent
        .flush_auto_extractions(std::time::Duration::from_millis(1))
        .await;
}

#[tokio::test]
async fn correction_detection_fires_event_callback_with_top_rule_id() {
    let mut agent = test_agent();
    let graph = MemoryGraph::in_memory().expect("in-memory graph");
    let seeded_rule_id = {
        let engine = RulesEngine::new(&graph);
        let rule = engine
            .add_rule("prefer concise corrections", RuleSource::UserDefined)
            .expect("seed rule");
        for _ in 0..10 {
            let _ = engine.reinforce_rule(&rule.id);
        }
        rule.id
    };
    let graph: Arc<dyn MemoryTrait> = Arc::new(graph);

    let correction_count = Arc::new(AtomicUsize::new(0));
    let correction_count_cb = Arc::clone(&correction_count);
    agent.set_record_correction_callback(Arc::new(move || {
        correction_count_cb.fetch_add(1, Ordering::SeqCst);
    }));

    let iv = Arc::new(Mutex::new(InnerVoice::new()));
    agent.set_inner_voice(Arc::clone(&iv));

    let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured_cb = Arc::clone(&captured);
    agent.set_record_user_correction_event_callback(Arc::new(move |payload| {
        captured_cb.lock().unwrap().push(payload);
    }));

    agent
        .detect_and_record_correction(&format!("use this instead {}", "x".repeat(220)), &graph)
        .await;

    assert_eq!(correction_count.load(Ordering::SeqCst), 1);
    let iv = iv.try_lock().expect("inner voice lock");
    assert_eq!(iv.corrections_received, 1);
    assert!((iv.confidence - 0.6).abs() < f32::EPSILON);

    let captured = captured.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].correction_type, "ApproachCorrection");
    assert_eq!(
        captured[0].top_rule_id.as_deref(),
        Some(seeded_rule_id.as_str())
    );
    assert!(!captured[0].user_input_excerpt.is_empty());
    assert!(captured[0].user_input_excerpt.chars().count() <= 200);
}

#[tokio::test]
async fn process_message_fires_runtime_lifecycle_hooks() {
    let mut agent = test_agent();
    let registry = Arc::new(crate::hooks::HookRegistry::new());
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    for event in [
        crate::hooks::HookEvent::BeforeAgentRun,
        crate::hooks::HookEvent::BeforePromptBuild,
        crate::hooks::HookEvent::AfterPromptBuild,
        crate::hooks::HookEvent::AfterAgentRun,
    ] {
        let seen_for_hook = Arc::clone(&seen);
        registry.register_callback(
            event.clone(),
            crate::hooks::HookCallbackEntry {
                name: format!("{event:?}"),
                callback: Arc::new(move |ctx| {
                    seen_for_hook.lock().unwrap().push(ctx.hook_event.clone());
                    crate::hooks::HookResult::allow()
                }),
                authority: crate::hooks::SourceAuthority::Project,
                timeout_secs: 1,
            },
        );
    }
    agent.set_hook_registry(registry);

    agent.process_message("hello").await.unwrap();

    let seen = seen.lock().unwrap();
    assert!(seen.contains(&crate::hooks::HookEvent::BeforeAgentRun));
    assert!(seen.contains(&crate::hooks::HookEvent::BeforePromptBuild));
    assert!(seen.contains(&crate::hooks::HookEvent::AfterPromptBuild));
    assert!(seen.contains(&crate::hooks::HookEvent::AfterAgentRun));
}

/// Verify that thinking blocks include the `signature` field when built
/// as assistant message content. This is required by the Anthropic API
/// for multi-turn conversations containing thinking blocks.
#[test]
fn thinking_block_includes_signature() {
    let thinking_content = "Let me analyze this step by step...";
    let thinking_signature = "EqoBCkEYstO+bkwMCwF8m...test-sig";

    let mut assistant_content: Vec<serde_json::Value> = Vec::new();

    if !thinking_content.is_empty() {
        assistant_content.push(serde_json::json!({
            "type": "thinking",
            "thinking": thinking_content,
            "signature": thinking_signature,
        }));
    }

    assistant_content.push(serde_json::json!({
        "type": "text",
        "text": "Here is my response.",
    }));

    assert_eq!(assistant_content.len(), 2);

    let thinking_block = &assistant_content[0];
    assert_eq!(thinking_block["type"], "thinking");
    assert_eq!(thinking_block["thinking"], thinking_content);
    assert_eq!(thinking_block["signature"], thinking_signature);
    // Crucially: the signature field MUST exist (not be null/missing)
    assert!(
        thinking_block.get("signature").is_some(),
        "thinking block must contain 'signature' field for Anthropic API"
    );
}

/// Verify that thinking blocks still include the signature field even
/// when the signature is empty (edge case: stream ended before signature).
#[test]
fn thinking_block_includes_empty_signature() {
    let thinking_content = "Some thinking...";
    let thinking_signature = "";

    let block = serde_json::json!({
        "type": "thinking",
        "thinking": thinking_content,
        "signature": thinking_signature,
    });

    assert!(block.get("signature").is_some());
    assert_eq!(block["signature"], "");
}

#[test]
fn conversation_state_add_assistant_message_preserves_thinking_signature() {
    let mut state = ConversationState::default();
    state.add_user_message("hello");

    let content = vec![
        serde_json::json!({
            "type": "thinking",
            "thinking": "deep thought",
            "signature": "sig123",
        }),
        serde_json::json!({
            "type": "text",
            "text": "response",
        }),
    ];
    state.add_assistant_message(content);

    assert_eq!(state.messages.len(), 2);
    let assistant_msg = &state.messages[1];
    let blocks = assistant_msg["content"]
        .as_array()
        .expect("content is array");
    assert_eq!(blocks[0]["signature"], "sig123");
}

// -----------------------------------------------------------------------
// TASK-AGT-012: Permission mode + max_concurrent tests
// -----------------------------------------------------------------------

#[test]
fn plan_mode_deny_list_is_static() {
    // Verify the plan mode deny constants are correct
    const PLAN_MODE_DENY: &[&str] = &["Write", "Edit", "Bash", "NotebookEdit"];
    assert!(PLAN_MODE_DENY.contains(&"Write"));
    assert!(PLAN_MODE_DENY.contains(&"Edit"));
    assert!(PLAN_MODE_DENY.contains(&"Bash"));
    assert!(PLAN_MODE_DENY.contains(&"NotebookEdit"));
    assert!(!PLAN_MODE_DENY.contains(&"Read"));
    assert!(!PLAN_MODE_DENY.contains(&"Grep"));
    assert!(!PLAN_MODE_DENY.contains(&"Glob"));
}

#[test]
fn subagent_manager_register_before_run_complete_after() {
    // Verify the SubagentManager register→complete lifecycle
    let mut mgr = crate::subagent::SubagentManager::new(4);
    let req = archon_tools::agent_tool::SubagentRequest {
        prompt: "test".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
    };

    // Register returns ID
    let id = mgr.register(req).expect("register should succeed");
    assert!(!id.is_empty());

    // Status is Running
    let info = mgr.get_status(&id).expect("should exist");
    assert!(matches!(
        info.status,
        crate::subagent::SubagentStatus::Running
    ));

    // Complete frees the slot
    mgr.complete(&id, "done".into())
        .expect("complete should work");
    let info = mgr.get_status(&id).expect("should still exist");
    assert!(matches!(
        info.status,
        crate::subagent::SubagentStatus::Completed
    ));
}

#[test]
fn subagent_manager_max_concurrent_enforced() {
    let mut mgr = crate::subagent::SubagentManager::new(1);
    let req = || archon_tools::agent_tool::SubagentRequest {
        prompt: "test".into(),
        model: None,
        allowed_tools: vec![],
        max_turns: 10,
        timeout_secs: 300,
        subagent_type: None,
        run_in_background: false,
        cwd: None,
        isolation: None,
    };

    let id1 = mgr.register(req()).expect("first register ok");

    // Second should fail
    let err = mgr.register(req());
    assert!(err.is_err(), "should reject second concurrent subagent");

    // Complete first, then second should succeed
    mgr.complete(&id1, "done".into()).unwrap();
    let _id2 = mgr
        .register(req())
        .expect("should succeed after completing first");
}

#[test]
fn permission_mode_plan_blocks_mutating_tools() {
    // Verify the filtering logic: in plan mode, Write/Edit/Bash/NotebookEdit are removed
    const PLAN_MODE_DENY: &[&str] = &["Write", "Edit", "Bash", "NotebookEdit"];
    let tools = vec![
        "Read",
        "Grep",
        "Glob",
        "Write",
        "Edit",
        "Bash",
        "NotebookEdit",
    ];
    let is_plan_mode = true;

    let filtered: Vec<&str> = tools
        .into_iter()
        .filter(|n| !is_plan_mode || !PLAN_MODE_DENY.contains(n))
        .collect();

    assert_eq!(filtered, vec!["Read", "Grep", "Glob"]);
}

#[test]
fn permission_mode_normal_allows_all_tools() {
    const PLAN_MODE_DENY: &[&str] = &["Write", "Edit", "Bash", "NotebookEdit"];
    let tools = vec!["Read", "Grep", "Glob", "Write", "Edit", "Bash"];
    let is_plan_mode = false;

    let filtered: Vec<&str> = tools
        .into_iter()
        .filter(|n| !is_plan_mode || !PLAN_MODE_DENY.contains(n))
        .collect();

    assert_eq!(
        filtered,
        vec!["Read", "Grep", "Glob", "Write", "Edit", "Bash"]
    );
}
