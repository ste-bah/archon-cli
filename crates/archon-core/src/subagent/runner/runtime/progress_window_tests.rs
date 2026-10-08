//! Issue 288 on the REAL runner path: `SubagentRunner::run` under a host
//! timeout of 7200 s is bounded by a no-progress window that only novel
//! activity renews, never by a total. A scripted provider takes 500 s per
//! turn (paused time), so a run of many turns passes 7200 s in total.

use std::collections::VecDeque;
use std::sync::Mutex;

use archon_llm::identity::IdentityMode;
use archon_llm::provider::{LlmError, LlmResponse, ModelInfo, ProviderFeature};
use archon_llm::types::Usage;
use archon_tools::host_timeout::{HostTimeout, scope};
use archon_tools::subagent_dispatch_clock::{DispatchClock, scope_session};
use archon_tools::tool::{PermissionLevel, Tool, ToolCapability, ToolResult};

use super::*;
use crate::agent::AgentConfig;

const WINDOW: u64 = 7_200;
const TURN: Duration = Duration::from_secs(500);

/// One scripted turn: optional text, then optional `Read` arguments.
type Turn = (Option<&'static str>, Option<String>);

/// A provider that answers each request after `TURN`, from a script; the
/// last scripted turn repeats for as long as the session keeps asking.
struct ScriptedProvider {
    turns: Mutex<VecDeque<Turn>>,
}

impl ScriptedProvider {
    fn new(turns: Vec<Turn>) -> Arc<Self> {
        Arc::new(Self {
            turns: Mutex::new(turns.into()),
        })
    }

    fn next(&self) -> Turn {
        let mut turns = self.turns.lock().unwrap();
        if turns.len() > 1 {
            turns.pop_front().unwrap()
        } else {
            turns.front().cloned().unwrap()
        }
    }
}

fn events((text, read): Turn) -> Vec<StreamEvent> {
    let mut events = vec![StreamEvent::MessageStart {
        id: "msg".into(),
        model: "mock".into(),
        usage: Usage::default(),
    }];
    let mut index = 0;
    if let Some(text) = text {
        events.extend([
            StreamEvent::ContentBlockStart {
                index,
                block_type: ContentBlockType::Text,
                tool_use_id: None,
                tool_name: None,
            },
            StreamEvent::TextDelta {
                index,
                text: text.into(),
            },
            StreamEvent::ContentBlockStop { index },
        ]);
        index += 1;
    }
    if let Some(arguments) = read {
        events.extend([
            StreamEvent::ContentBlockStart {
                index,
                block_type: ContentBlockType::ToolUse,
                tool_use_id: Some(format!("call-{index}")),
                tool_name: Some("Read".into()),
            },
            StreamEvent::InputJsonDelta {
                index,
                partial_json: arguments,
            },
            StreamEvent::ContentBlockStop { index },
        ]);
    }
    events.push(StreamEvent::MessageStop);
    events
}

#[async_trait::async_trait]
impl LlmProvider for ScriptedProvider {
    fn name(&self) -> &str {
        "scripted"
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
        tokio::time::sleep(TURN).await;
        let events = events(self.next());
        let (tx, rx) = tokio::sync::mpsc::channel(events.len() + 1);
        for event in events {
            tx.send(event).await.expect("send stream event");
        }
        Ok(rx)
    }
    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("tests use streaming")
    }
}

/// A read-only `Read` that answers instantly with fixed text.
struct FixedRead;

#[async_trait::async_trait]
impl Tool for FixedRead {
    fn name(&self) -> &str {
        "Read"
    }
    fn description(&self) -> &str {
        "fixed read"
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({ "type": "object" })
    }
    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        ToolResult::success("fn main() {}")
    }
    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
    fn capability(&self) -> ToolCapability {
        ToolCapability::HostLocal
    }
}

fn runner(provider: Arc<ScriptedProvider>, session: &str) -> SubagentRunner {
    let mut registry = crate::dispatch::ToolRegistry::new();
    registry.register(Box::new(FixedRead));
    let definitions = registry.tool_definitions();
    let context = ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        session_id: session.into(),
        mode: archon_tools::tool::AgentMode::Normal,
        ..Default::default()
    };
    SubagentRunner::new(
        provider,
        "You are a test subagent.".into(),
        definitions,
        Arc::new(registry),
        context,
        "mock-model".into(),
        10_000,
        300,
        Arc::new(AgentConfig::default()),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "test".into(),
            String::new(),
            String::new(),
        )),
    )
}

fn read(path: &str) -> Option<String> {
    Some(serde_json::json!({ "file_path": path }).to_string())
}

/// Run under a 7200 s host timeout with an admitted host dispatch clock fed
/// by the same runner; the result, the elapsed time and that clock.
async fn run(turns: Vec<Turn>, session: &str) -> (anyhow::Result<String>, u64, Arc<DispatchClock>) {
    let clock = DispatchClock::new();
    clock.admit();
    let runner = runner(ScriptedProvider::new(turns), session);
    let started = tokio::time::Instant::now();
    let result = scope_session(
        session,
        vec![Arc::clone(&clock)],
        scope(HostTimeout::Finite(WINDOW), runner.run("go")),
    )
    .await;
    (result, started.elapsed().as_secs(), clock)
}

fn stall_text(result: anyhow::Result<String>) -> String {
    let text = result.expect_err("a stalled session must stop").to_string();
    assert!(text.contains("without progress"), "{text}");
    assert!(text.contains("no-progress window of 7200s"), "{text}");
    // The host classifies it as its own call timeout: a resumable stop.
    assert!(text.contains("subagent timed out after"), "{text}");
    text
}

#[tokio::test(start_paused = true)]
async fn steady_novel_progress_runs_past_the_window_total() {
    let mut turns: Vec<Turn> = (0..30)
        .map(|step| (None, read(&format!("/src/file_{step}.rs"))))
        .collect();
    turns.push((Some("done"), None));
    let (result, elapsed, clock) = run(turns, "pw-steady").await;
    assert_eq!(result.expect("novel work is never cut"), "done");
    assert!(elapsed >= 15_000, "ran {elapsed}s, past 7200s in total");
    assert!(
        clock.elapsed() < Duration::from_secs(WINDOW),
        "host clock renewed"
    );
    let last = clock
        .last_progress()
        .expect("the host clock saw novel activity");
    assert!(last.contains("assistant text \"done\""), "{last}");
}

#[tokio::test(start_paused = true)]
async fn repeating_the_same_read_and_text_stops_at_the_window() {
    let turns = vec![(Some("Let me check again."), read("/src/same.rs"))];
    let (result, elapsed, clock) = run(turns, "pw-repeat").await;
    let text = stall_text(result);
    // Turn 1 is novel at 500 s; every later turn repeats it exactly.
    assert_eq!(elapsed, 500 + WINDOW, "{text}");
    assert!(
        text.contains("last novel activity: turn 1: new tool call Read"),
        "{text}"
    );
    assert!(text.contains("/src/same.rs"), "{text}");
    let last = clock
        .last_progress()
        .expect("turn 1 renewed the host clock");
    assert!(text.contains(&last), "one signal for both: {last}");
}

#[tokio::test(start_paused = true)]
async fn alternating_two_identical_reads_stops_at_the_window() {
    let mut turns = Vec::new();
    for _ in 0..40 {
        turns.push((None, read("/src/a.rs")));
        turns.push((None, read("/src/b.rs")));
    }
    let (result, elapsed, _) = run(turns, "pw-alternate").await;
    let text = stall_text(result);
    assert_eq!(elapsed, 1_000 + WINDOW, "{text}");
    assert!(text.contains("turn 2: new tool call Read"), "{text}");
    assert!(text.contains("/src/b.rs"), "{text}");
}

/// Argument order is not novelty: the same call with its keys reordered.
#[tokio::test(start_paused = true)]
async fn reordered_arguments_are_the_same_call() {
    let turns = vec![
        (
            None,
            Some(r#"{"file_path":"/src/x.rs","limit":5}"#.to_string()),
        ),
        (
            None,
            Some(r#"{"limit":5,"file_path":"/src/x.rs"}"#.to_string()),
        ),
    ];
    let (result, elapsed, _) = run(turns, "pw-reorder").await;
    stall_text(result);
    assert_eq!(elapsed, 500 + WINDOW);
}
