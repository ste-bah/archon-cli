use super::*;
// Used only by `cfg(unix)` tests in this file. See #136.
#[cfg(unix)]
use archon_tools::bash::BashTool;

// ── v0.1.12: parallel tool dispatch regression test ──────────

/// A tool that reports how many copies of itself were running at once.
///
/// The delay is only there to hold the overlap window open; what the parallel
/// dispatch test reads is `peak`, not the clock.
struct SleeperTool {
    name: String,
    delay_ms: u64,
    inflight: Arc<std::sync::atomic::AtomicUsize>,
    peak: Arc<std::sync::atomic::AtomicUsize>,
}

struct RiskySubagentTool {
    executions: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait::async_trait]
impl Tool for RiskySubagentTool {
    fn name(&self) -> &str {
        "RiskySubagent"
    }

    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }

    fn description(&self) -> &str {
        "subagent admission test"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type": "object"})
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        self.executions
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ToolResult::success("executed")
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Risky
    }
}

#[async_trait::async_trait]
impl Tool for SleeperTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }
    fn description(&self) -> &str {
        "test sleeper"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({})
    }
    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        use std::sync::atomic::Ordering;
        let now = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        self.inflight.fetch_sub(1, Ordering::SeqCst);
        ToolResult::success(format!("done:{}", self.name))
    }
    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_tool_dispatch_concurrent_and_order_preserved() {
    let inflight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    for (name, delay_ms) in [("sleeper-A", 400), ("sleeper-B", 200), ("sleeper-C", 300)] {
        registry.register(Box::new(SleeperTool {
            name: name.into(),
            delay_ms,
            inflight: Arc::clone(&inflight),
            peak: Arc::clone(&peak),
        }));
    }
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
                    ..Usage::default()
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

    let result = runner.run("run all three").await.unwrap();

    // The subagent ran turn 1 (3 tool_use dispatched in parallel)
    // then turn 2 (text "all done"). If dispatch had failed, the
    // subagent would have returned an error. "all done" means the
    // loop completed cleanly.
    assert_eq!(result, "all done");

    // Concurrency, observed rather than inferred from a stopwatch. This used to
    // assert the three 200-400ms sleeps finished inside 900ms, with 1.5x
    // headroom over the serial sum — a margin a loaded CI box eats without any
    // regression in dispatch. The tools now report how many of them were in
    // flight at once, which serial dispatch cannot fake at any speed.
    assert_eq!(
        peak.load(std::sync::atomic::Ordering::SeqCst),
        3,
        "expected all three tool calls in flight together; serial dispatch peaks at 1"
    );
    assert_eq!(
        inflight.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "every dispatched tool must have finished before the turn completed"
    );
}

#[tokio::test]
async fn subagent_tool_round_admits_provider_tool_use_id_before_execution() {
    let provider = Arc::new(MockProvider::new(vec![
        tool_use_response("provider-tool-use-1", "RiskySubagent", "{}"),
        text_response("done"),
    ]));
    let executions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(RiskySubagentTool {
        executions: Arc::clone(&executions),
    }));
    let registry = Arc::new(registry);
    let admissions = Arc::new(std::sync::Mutex::new(Vec::new()));
    let admissions_for_callback = Arc::clone(&admissions);
    let outcomes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let outcomes_for_callback = Arc::clone(&outcomes);
    let runner = SubagentRunner::new(
        provider,
        "test".into(),
        registry.tool_definitions(),
        registry,
        ToolContext {
            session_id: "subagent-session".into(),
            tool_run_parent_action_id: Some("parent-1".into()),
            tool_run_admission: Some(Arc::new(move |request| {
                admissions_for_callback
                    .lock()
                    .unwrap()
                    .push((request.tool_use_id, request.attempt));
                archon_tools::tool::ToolRunAdmission::Allowed
            })),
            tool_run_outcome: Some(Arc::new(move |outcome| {
                outcomes_for_callback
                    .lock()
                    .unwrap()
                    .push((outcome.tool_use_id, outcome.attempt));
            })),
            ..ToolContext::default()
        },
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

    let result = runner.run("run risky tool").await.unwrap();

    assert_eq!(result, "done");
    assert_eq!(executions.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        *admissions.lock().unwrap(),
        vec![("provider-tool-use-1".into(), 0)]
    );
    assert_eq!(
        *outcomes.lock().unwrap(),
        vec![("provider-tool-use-1".into(), 0)]
    );
}

/// The subagent's own wall-clock cap for this run, named so the constructor
/// argument and the assertion below cannot drift apart. It must stay well under
/// the 60s `BashTool` timeout, because the whole point is proving which of the
/// two deadlines ended the run.
#[cfg(unix)]
const SUBAGENT_WALL_CLOCK_CAP_SECS: u64 = 2;

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_round_timeout_kills_bash_process_group() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("sleep.pid");
    let command = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
    let input = serde_json::json!({"command": command}).to_string();
    let provider = Arc::new(MockProvider::new(vec![tool_use_response(
        "bash-timeout",
        "Bash",
        &input,
    )]));
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(BashTool {
        timeout_secs: 60,
        max_output_bytes: 1024,
        provider_env: None,
        ..Default::default()
    }));
    let registry = Arc::new(registry);
    let runner = SubagentRunner::new(
        provider,
        "test".into(),
        registry.tool_definitions(),
        registry,
        ToolContext {
            working_dir: dir.path().to_path_buf(),
            session_id: "tool-round-timeout".into(),
            ..ToolContext::default()
        },
        "mock".into(),
        5,
        SUBAGENT_WALL_CLOCK_CAP_SECS,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test".into(),
            String::new(),
            String::new(),
        )),
    );

    // The error itself says which deadline fired and in which phase, and no
    // amount of machine load rewrites it. The outer timeout is a liveness guard
    // for the case where the 5s cap does not fire at all — set an order of
    // magnitude above it precisely so it is never the thing being measured.
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        runner.run("run the command"),
    )
    .await
    .expect("the 5s wall-clock cap never fired; the tool round was left running")
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("during tool round"), "{message}");
    // Built from the same constant the runner was given, so a change to the cap
    // cannot leave this asserting a number nothing configures. It previously
    // read `5s` — the max-turns argument, not the cap — which made the test
    // fail everywhere it actually ran. It only ever ran on unix, and the
    // Windows-only verification that cleared it could not compile this file.
    let expected_cap = format!("(cap: {SUBAGENT_WALL_CLOCK_CAP_SECS}s)");
    assert!(
        message.contains(&expected_cap),
        "the run must end on the subagent's own cap, not the tool's 60s one: {message}"
    );
    let pid = std::fs::read_to_string(pid_file).unwrap();
    assert_process_stopped(pid.trim()).await;
}

#[cfg(unix)]
async fn assert_process_stopped(pid: &str) {
    for _ in 0..20 {
        let alive = std::process::Command::new("kill")
            .args(["-0", pid])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if !alive {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("timed-out tool descendant survived: pid={pid}");
}
