//! Production dispatch results and the actual print-mode consumer/return path.
use super::tool_types::PreflightResult;
use super::*;
use archon_tools::tool::{PermissionLevel, Tool, WorkingTreeEffect};
use std::time::Duration;

struct ReadOnlyTool;
#[async_trait::async_trait]
impl Tool for ReadOnlyTool {
    fn name(&self) -> &str {
        "HookRead"
    }
    fn description(&self) -> &str {
        "read-only test tool"
    }
    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object"})
    }
    fn permission_level(&self, _: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
    async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
        ToolResult::success("read completed")
    }
}

fn print_config(format: crate::output_format::OutputFormat) -> crate::print_mode::PrintModeConfig {
    crate::print_mode::PrintModeConfig {
        query: "finish".into(),
        output_format: format,
        input_format: crate::input_format::InputFormat::Text,
        max_turns: None,
        max_budget_usd: None,
        no_session_persistence: true,
        json_schema: None,
    }
}

fn configured_agent(
    command: &str,
    background: bool,
    dir: &std::path::Path,
) -> (Agent, tokio::sync::mpsc::Receiver<TimestampedEvent>) {
    let hooks = Arc::new(crate::hooks::HookRegistry::new());
    hooks.register_session_hook(
        "dispatch",
        crate::hooks::HookEvent::PostToolUse,
        serde_json::from_value(serde_json::json!({
            "type":"command", "command":command, "async":background,
            "timeout":1, "on_failure":"allow"
        }))
        .unwrap(),
    );
    let mut agent = super::tests::test_agent();
    let (tx, events) = tokio::sync::mpsc::channel(32);
    agent.event_tx = tx;
    agent.config.working_dir = dir.to_path_buf();
    agent.config.session_id = "dispatch".into();
    agent.set_hook_registry(hooks);
    (agent, events)
}

async fn sync_dispatch(command: &str, format: crate::output_format::OutputFormat) {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, mut events) = configured_agent(command, false, dir.path());
    // The succeeding hook must not replace or drop the stop diagnostic.
    agent.hook_registry.as_ref().unwrap().register_session_hook(
        "dispatch", crate::hooks::HookEvent::PostToolUse,
        serde_json::from_value(serde_json::json!({
            "type":"command", "command": r#"printf '%s' '{"outcome":"success","additional_context":"next hook ran","updated_mcp_tool_output":"replacement"}'"#,
            "timeout":1
        })).unwrap(),
    );
    let pre = PreflightResult {
        tool_name: "HookRead".into(),
        tool_id: "read-1".into(),
        input: serde_json::json!({}),
        tool_arc: Arc::new(ReadOnlyTool),
        file_path: None,
        filesystem_effect: WorkingTreeEffect::None,
        filesystem_before: None,
        sandbox_prechecked: true,
    };
    let ctx = ToolContext::default();
    let results = agent
        .dispatch_allowed_tools(std::slice::from_ref(&pre), &ctx)
        .await;
    let stop = agent.postprocess_tools(&[pre], results, &ctx, "test").await;
    assert!(stop.is_none(), "allowing stop prevented continuation");
    let mut completed = None;
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event.inner, AgentEvent::Error(_)),
            "hook changed agent turn state"
        );
        if let AgentEvent::ToolCallComplete { ref result, .. } = event.inner {
            assert!(!result.is_error, "allowing stop failed the tool");
            assert!(result.content.contains("replacement"));
            assert!(result.content.contains("next hook ran"));
            assert!(
                result.content.contains("[Hook Warning] PostToolUse: "),
                "{}",
                result.content
            );
            assert!(result.content.contains("no progress"), "{}", result.content);
            // The model-visible line names the hook by id, never by its
            // full command line; that stays in the operator log.
            let id = crate::hooks::compute_hook_id(
                &crate::hooks::HookEvent::PostToolUse,
                &crate::hooks::HookCommandType::Command,
                command,
                None,
            );
            assert!(result.content.contains(&id), "{}", result.content);
            assert!(!result.content.contains(command), "{}", result.content);
            assert_eq!(result.content.matches("[Hook Warning]").count(), 1);
            completed = Some(event);
        }
    }
    let completed = completed.expect("dispatch must publish tool completion");
    let context = serde_json::to_string(&agent.state.messages).unwrap();
    assert!(
        context.contains("no progress"),
        "agent context lost warning"
    );
    // Feed the production tool-completion event to the real print consumer.
    agent.event_tx.send(completed).await.unwrap();
    let code = tokio::time::timeout(
        Duration::from_secs(8),
        crate::print_mode::run_print_mode(
            print_config(format),
            &crate::config::ArchonConfig::default(),
            &mut agent,
            events,
        ),
    )
    .await
    .expect("print mode must return");
    assert_eq!(code, crate::print_mode::EXIT_SUCCESS);
}

fn sync_child(test_name: &str, command: &str, format: crate::output_format::OutputFormat) {
    const CHILD_ENV: &str = "ARCHON_HOOK_DIAGNOSTIC_TEST_CHILD";
    if std::env::var_os(CHILD_ENV).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(sync_dispatch(command, format));
        return;
    }
    let exact = format!("agent::hook_progress_delivery_tests::{test_name}");
    let output = archon_shell::spawn::command(std::env::current_exe().unwrap())
        .args([&exact, "--exact", "--test-threads=1", "--nocapture"])
        .env(CHILD_ENV, "1")
        .output()
        .expect("spawn-helper child test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed"),
        "child failed: {}\n{stdout}\n{stderr}",
        output.status
    );
    assert!(stderr.contains("[tool:HookRead]"), "{stderr}");
    assert!(
        stderr.contains("[Hook Warning] PostToolUse: ") && stderr.contains("no progress"),
        "{stderr}"
    );
    assert_eq!(stderr.matches("[Hook Warning]").count(), 1, "{stderr}");
    if format == crate::output_format::OutputFormat::StreamJson {
        assert!(
            stdout.contains("no progress") && stdout.contains("tool_result"),
            "{stdout}"
        );
    }
}

#[test]
fn sync_silent_stop_text() {
    sync_child(
        "sync_silent_stop_text",
        "sleep 600",
        crate::output_format::OutputFormat::Text,
    );
}
#[test]
fn sync_output_then_stop_json() {
    sync_child(
        "sync_output_then_stop_json",
        "echo progress; sleep 600",
        crate::output_format::OutputFormat::Json,
    );
}
#[test]
fn sync_eof_descendant_stop_stream_json() {
    sync_child(
        "sync_eof_descendant_stop_stream_json",
        "sleep 600 </dev/null >/dev/null 2>&1 & exit 0",
        crate::output_format::OutputFormat::StreamJson,
    );
}

async fn async_print_returns(
    event: crate::hooks::HookEvent,
    format: crate::output_format::OutputFormat,
) {
    let dir = tempfile::tempdir().unwrap();
    let (mut agent, events) = configured_agent("exit 0", true, dir.path());
    agent.hook_registry.as_ref().unwrap().register_session_hook(
        "dispatch", event.clone(),
        serde_json::from_value(serde_json::json!({
            "type":"command", "command":"echo $$ > hook.pid; for n in $(seq 1 60); do echo progress; sleep 0.2; done",
            "async":true, "timeout":1, "on_failure":"allow"
        })).unwrap(),
    );
    let aggregate = agent
        .fire_hook(event, serde_json::json!({"tool_id":"read-1"}))
        .await;
    assert!(aggregate.nonblocking_errors.is_empty());
    // Ensure the background child really started and is live at print shutdown.
    tokio::time::timeout(Duration::from_secs(8), async {
        while !dir.path().join("hook.pid").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let code = tokio::time::timeout(
        Duration::from_secs(4),
        crate::print_mode::run_print_mode(
            print_config(format),
            &crate::config::ArchonConfig::default(),
            &mut agent,
            events,
        ),
    )
    .await
    .expect("live async hook retained the foreground event sender");
    assert_eq!(code, crate::print_mode::EXIT_SUCCESS);
    let pid: i32 = std::fs::read_to_string(dir.path().join("hook.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        unsafe { libc::kill(pid, 0) },
        0,
        "print mode waited for background hook"
    );
    // Runtime teardown cancels the background task; owned group cleanup kills it.
}

#[tokio::test]
async fn async_post_tool_does_not_hold_text_print_open() {
    async_print_returns(
        crate::hooks::HookEvent::PostToolUse,
        crate::output_format::OutputFormat::Text,
    )
    .await;
}
#[tokio::test]
async fn async_session_start_does_not_hold_json_print_open() {
    async_print_returns(
        crate::hooks::HookEvent::SessionStart,
        crate::output_format::OutputFormat::Json,
    )
    .await;
}
#[tokio::test]
async fn async_subagent_stop_does_not_hold_stream_print_open() {
    async_print_returns(
        crate::hooks::HookEvent::SubagentStop,
        crate::output_format::OutputFormat::StreamJson,
    )
    .await;
}

fn async_stop(command: &str) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let (mut agent, mut events) = configured_agent(command, true, dir.path());
            let start = std::time::Instant::now();
            let aggregate = agent
                .fire_hook(crate::hooks::HookEvent::PostToolUse, serde_json::json!({}))
                .await;
            assert!(
                start.elapsed() < Duration::from_secs(1),
                "async launch awaited hook"
            );
            assert!(aggregate.nonblocking_errors.is_empty());
            let pid = tokio::time::timeout(Duration::from_secs(8), async {
                loop {
                    if let Ok(raw) = std::fs::read_to_string(dir.path().join("hook.pid")) {
                        if let Ok(pid) = raw.trim().parse::<i32>() {
                            break pid;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("background child did not start");
            tokio::time::timeout(Duration::from_secs(8), async {
                while unsafe { libc::kill(pid, 0) } == 0 {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("no-progress stop did not clean up child");
            // Let any result sink enqueue its event before checking the channel.
            tokio::time::sleep(Duration::from_millis(100)).await;
            while let Ok(event) = events.try_recv() {
                assert!(
                    !matches!(event.inner, AgentEvent::Error(_)),
                    "async result reached the agent channel: {:?}",
                    event.inner
                );
            }
            agent.close_event_channel();
            assert!(
                tokio::time::timeout(Duration::from_secs(1), events.recv())
                    .await
                    .unwrap()
                    .is_none()
            );
        });
}

macro_rules! async_warn_test {
    ($name:ident, $command:literal) => {
        #[test]
        #[tracing_test::traced_test]
        fn $name() {
            async_stop($command);
            logs_assert(|lines| {
                let stops: Vec<_> = lines
                    .iter()
                    .filter(|line| line.contains("no progress") && line.contains($command))
                    .collect();
                if stops.len() != 1 {
                    return Err(format!("expected one stop WARN, got {stops:?}"));
                }
                let line = stops[0];
                if !line.contains("WARN") || !line.contains("event=PostToolUse") {
                    return Err(format!("stop lacks WARN/event/reason/hook: {line}"));
                }
                Ok(())
            });
        }
    };
}
async_warn_test!(async_warn_silent_stop, "echo $$ > hook.pid; exec sleep 600");
async_warn_test!(
    async_warn_output_then_stop,
    "echo $$ > hook.pid; echo progress; exec sleep 600"
);
async_warn_test!(
    async_warn_eof_descendant_stop,
    "sleep 600 </dev/null >/dev/null 2>&1 & echo $! > hook.pid; exit 0"
);
