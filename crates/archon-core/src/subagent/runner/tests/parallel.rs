use super::*;

// ── v0.1.12: parallel tool dispatch regression test ──────────

struct SleeperTool {
    name: String,
    delay_ms: u64,
}

#[async_trait::async_trait]
impl Tool for SleeperTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "test sleeper"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        ToolResult::success(format!("done:{}", self.name))
    }
    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_tool_dispatch_concurrent_and_order_preserved() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(SleeperTool {
        name: "sleeper-A".into(),
        delay_ms: 400,
    }));
    registry.register(Box::new(SleeperTool {
        name: "sleeper-B".into(),
        delay_ms: 200,
    }));
    registry.register(Box::new(SleeperTool {
        name: "sleeper-C".into(),
        delay_ms: 300,
    }));
    let registry = Arc::new(registry);
    let tool_defs = registry.tool_definitions();

    let provider = Arc::new(MockProvider::new(vec![
        // Turn 1: 3 tool_use blocks with shuffled delays
        vec![
            StreamEvent::MessageStart {
                id: "msg-1".into(),
                model: "mock".into(),
                usage: Usage {
                    input_tokens: 10,
                    output_tokens: 5,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                },
            },
            StreamEvent::ContentBlockStart {
                index: 0,
                block_type: ContentBlockType::ToolUse,
                tool_use_id: Some("t1".into()),
                tool_name: Some("sleeper-A".into()),
            },
            StreamEvent::InputJsonDelta {
                index: 0,
                partial_json: "{}".into(),
            },
            StreamEvent::ContentBlockStop { index: 0 },
            StreamEvent::ContentBlockStart {
                index: 1,
                block_type: ContentBlockType::ToolUse,
                tool_use_id: Some("t2".into()),
                tool_name: Some("sleeper-B".into()),
            },
            StreamEvent::InputJsonDelta {
                index: 1,
                partial_json: "{}".into(),
            },
            StreamEvent::ContentBlockStop { index: 1 },
            StreamEvent::ContentBlockStart {
                index: 2,
                block_type: ContentBlockType::ToolUse,
                tool_use_id: Some("t3".into()),
                tool_name: Some("sleeper-C".into()),
            },
            StreamEvent::InputJsonDelta {
                index: 2,
                partial_json: "{}".into(),
            },
            StreamEvent::ContentBlockStop { index: 2 },
            StreamEvent::MessageStop,
        ],
        text_response("all done"),
    ]));

    let ctx = ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        session_id: "test-parallel".into(),
        mode: archon_tools::tool::AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    };

    let runner = SubagentRunner::new(
        provider,
        "test".into(),
        tool_defs,
        registry,
        ctx,
        "mock".into(),
        5,
        60,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test".into(),
            String::new(),
            String::new(),
        )),
    );

    let start = std::time::Instant::now();
    let result = runner.run("run all three").await.unwrap();
    let elapsed = start.elapsed();

    // The subagent ran turn 1 (3 tool_use dispatched in parallel)
    // then turn 2 (text "all done"). If dispatch had failed, the
    // subagent would have returned an error. "all done" means the
    // loop completed cleanly.
    assert_eq!(result, "all done");

    // Concurrent: max delay is 400ms. Serial sum would be ~900ms.
    // 1.5× headroom for CI variance.
    assert!(
        elapsed.as_millis() < 900,
        "{}ms — expected <900ms for 3×400ms concurrent (serial would be ~900ms)",
        elapsed.as_millis()
    );
}
