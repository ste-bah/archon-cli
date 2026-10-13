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
use archon_tools::tool::{PermissionLevel, Tool, ToolCapability, ToolContext, ToolResult};

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
    tool: &'static str,
}

impl ScriptedProvider {
    fn new(turns: Vec<Turn>) -> Arc<Self> {
        Self::calling(turns, "Read")
    }

    /// The same script, calling `tool` instead of `Read`.
    fn calling(turns: Vec<Turn>, tool: &'static str) -> Arc<Self> {
        Arc::new(Self {
            turns: Mutex::new(turns.into()),
            tool,
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

fn events((text, read): Turn, tool: &str) -> Vec<StreamEvent> {
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
                tool_name: Some(tool.into()),
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
        let events = events(self.next(), self.tool);
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
struct FixedRead {
    refusal: bool,
}

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
        if self.refusal {
            ToolResult::refusal("read-only inspection refused")
        } else {
            ToolResult::success("fn main() {}")
        }
    }
    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
    fn capability(&self) -> ToolCapability {
        ToolCapability::HostLocal
    }
}

fn runner(provider: Arc<ScriptedProvider>, session: &str) -> SubagentRunner {
    runner_with_read(provider, session, false)
}

fn runner_with_read(
    provider: Arc<ScriptedProvider>,
    session: &str,
    refusal: bool,
) -> SubagentRunner {
    let mut registry = crate::dispatch::ToolRegistry::new();
    registry.register(Box::new(FixedRead { refusal }));
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

fn runner_with_read_ceiling(provider: Arc<ScriptedProvider>, session: &str) -> SubagentRunner {
    let mut registry = crate::dispatch::ToolRegistry::new();
    registry.register(Box::new(FixedRead { refusal: false }));
    let definitions = registry.tool_definitions();
    let mut context = ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        session_id: session.into(),
        mode: archon_tools::tool::AgentMode::Normal,
        ..Default::default()
    };
    context.workflow_read_guard = Some(Arc::new(
        archon_tools::workflow_read_guard::WorkflowReadGuard::shell_only(
            &archon_tools::workflow_read_guard::WorkflowReadGuardSettings {
                read_only_soft_call_ceiling: 0,
                read_only_hard_call_ceiling: 1,
                ..Default::default()
            },
        ),
    ));
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
async fn different_text_form_tool_calls_do_not_renew_the_window() {
    let turns = [
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/0.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/1.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/2.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/3.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/4.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/5.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/6.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/7.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/8.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/9.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/10.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/11.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/12.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/13.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/14.rs</arg_value></tool_call>",
        "<tool_call>Read<arg_key>file_path</arg_key><arg_value>/src/15.rs</arg_value></tool_call>",
    ];
    let (result, elapsed, _) = run(
        turns.into_iter().map(|text| (Some(text), None)).collect(),
        "pw-text-tool-call",
    )
    .await;
    let error = stall_text(result);

    assert_eq!(elapsed, WINDOW, "{error}");
    assert!(!error.contains("max turns"), "{error}");
}

#[tokio::test(start_paused = true)]
async fn refused_reads_do_not_renew_and_end_as_resumable_no_progress() {
    // Every call refused by a typed guard refusal, with no inspection
    // ceiling to withdraw the tools: only the window can end it.
    let mut turns = vec![(
        Some(r#"Read {"file_path":"/src/refused_0.rs"}"#),
        read("/src/refused_0.rs"),
    )];
    for offset in 1..20 {
        turns.push((None, read(&format!("/src/refused_{offset}.rs"))));
    }
    let clock = DispatchClock::new();
    clock.admit();
    let runner = runner_with_read(ScriptedProvider::new(turns), "pw-refused", true);
    let started = tokio::time::Instant::now();
    let result = scope_session(
        "pw-refused",
        vec![Arc::clone(&clock)],
        scope(HostTimeout::Finite(WINDOW), runner.run("go")),
    )
    .await;
    let elapsed = started.elapsed().as_secs();
    let error = stall_text(result);
    // Nothing was ever admitted, and text that only echoes the call is not
    // novel: the window never renewed. 14 rounds finished inside it.
    assert_eq!(elapsed, WINDOW, "{error}");
    assert!(error.contains("refused tool calls: 14"), "{error}");
    assert!(
        error.contains("last novel activity: none since the window opened"),
        "{error}"
    );
    // Issue 382: the host's clock carries the same count for its own cut.
    assert_eq!(clock.refused_calls(), 14);
    assert_eq!(
        clock.cut_text(),
        "last novel activity: none since the window opened; refused tool calls: 14"
    );
    // The host timeout counts typed refusals as refusals, not novel activity,
    // so they do not renew the window. Their reason text stays on the tool result.
}

/// Past the hard ceiling the refused rounds no longer wait for the window:
/// the no-progress stop ends the session with its count (Issue 382).
#[tokio::test(start_paused = true)]
async fn refused_reads_past_the_ceiling_end_as_no_progress_with_their_count() {
    let mut turns = vec![(None, read("/src/refused_0.rs"))];
    for offset in 1..20 {
        turns.push((None, read(&format!("/src/refused_{offset}.rs"))));
    }
    let clock = DispatchClock::new();
    clock.admit();
    let runner = runner_with_read_ceiling(ScriptedProvider::new(turns), "pw-refused-ceiling");
    let started = tokio::time::Instant::now();
    let result = scope_session(
        "pw-refused-ceiling",
        vec![Arc::clone(&clock)],
        scope(HostTimeout::Finite(WINDOW), runner.run("go")),
    )
    .await;
    let elapsed = started.elapsed().as_secs();
    let error = result.expect_err("refused rounds end the session").to_string();
    assert!(error.starts_with(archon_tools::NO_PROGRESS_STOP_MARKER), "{error}");
    assert!(error.contains("refused tool calls: 8"), "{error}");
    assert!(error.contains("last novel activity: turn 1: new tool call Read"), "{error}");
    // Turn 1 admitted, then 8 refused rounds: 9 turns of 500 s.
    assert_eq!(elapsed, 9 * 500, "{error}");
    assert_eq!(clock.refused_calls(), 8);
}

#[tokio::test]
async fn hard_ceiling_refusal_is_typed() {
    let guard = Arc::new(
        archon_tools::workflow_read_guard::WorkflowReadGuard::shell_only(
            &archon_tools::workflow_read_guard::WorkflowReadGuardSettings {
                read_only_soft_call_ceiling: 0,
                read_only_hard_call_ceiling: 1,
                ..Default::default()
            },
        ),
    );
    let context = ToolContext {
        working_dir: std::env::current_dir().unwrap_or_default(),
        workflow_read_guard: Some(guard),
        ..Default::default()
    };
    let tool = FixedRead { refusal: false };
    let input = serde_json::json!({"file_path": "/src/ceiling.rs"});
    let admitted =
        crate::tool_run_admission::execute_tool_attempt(&tool, input.clone(), &context, false)
            .await;
    assert!(!admitted.is_guard_refusal());
    let refused =
        crate::tool_run_admission::execute_tool_attempt(&tool, input, &context, false).await;
    assert!(refused.is_guard_refusal(), "{}", refused.content);
    assert!(refused.content.contains("read ceiling reached:"));
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

// ---------------------------------------------------------------------------
// Issue 391: novelty is new CONTENT, not a new (path, offset) tuple. These run
// the real `Read` and `read-own-evidence` tools over real files.

/// A runner whose only tool is `tool`, working in `dir`.
fn runner_with_tool(
    provider: Arc<ScriptedProvider>,
    session: &str,
    dir: &std::path::Path,
    tool: Box<dyn Tool>,
) -> SubagentRunner {
    let mut registry = crate::dispatch::ToolRegistry::new();
    registry.register(tool);
    let definitions = registry.tool_definitions();
    let context = ToolContext {
        working_dir: dir.to_path_buf(),
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

async fn run_with_tool(
    turns: Vec<Turn>,
    session: &str,
    dir: &std::path::Path,
    name: &'static str,
    tool: Box<dyn Tool>,
) -> (anyhow::Result<String>, u64) {
    let clock = DispatchClock::new();
    clock.admit();
    let runner = runner_with_tool(ScriptedProvider::calling(turns, name), session, dir, tool);
    let started = tokio::time::Instant::now();
    let result = scope_session(
        session,
        vec![Arc::clone(&clock)],
        scope(HostTimeout::Finite(WINDOW), runner.run("go")),
    )
    .await;
    (result, started.elapsed().as_secs())
}

/// A real file of `lines` distinct lines.
fn numbered_file(dir: &std::path::Path, lines: usize) -> std::path::PathBuf {
    let path = dir.join("context.md");
    let text = (1..=lines)
        .map(|line| format!("line {line} of the author context"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, text).unwrap();
    path
}

fn slice(path: &std::path::Path, offset: usize, limit: usize) -> Option<String> {
    Some(serde_json::json!({ "file_path": path, "offset": offset, "limit": limit }).to_string())
}

#[tokio::test(start_paused = true)]
async fn paging_a_file_twice_renews_the_window_only_on_the_first_pass() {
    let dir = tempfile::tempdir().unwrap();
    let path = numbered_file(dir.path(), 105);
    // Pass 1: three 35-line slices cover the whole file. Pass 2: the same
    // file again in 30-line slices at new offsets — new arguments, no new
    // content. The last turn repeats for as long as the session asks.
    let mut turns: Vec<Turn> = [0, 35, 70]
        .into_iter()
        .map(|offset| (None, slice(&path, offset, 35)))
        .collect();
    turns.extend(
        [5, 35, 65, 95]
            .into_iter()
            .map(|offset| (None, slice(&path, offset, 30))),
    );
    let (result, elapsed) = run_with_tool(
        turns,
        "pw-391-page-twice",
        dir.path(),
        "Read",
        Box::new(archon_tools::file_read::ReadTool),
    )
    .await;
    let text = stall_text(result);
    assert!(
        text.contains("last novel activity: turn 3: new tool call Read"),
        "{text}"
    );
    assert_eq!(elapsed, 3 * 500 + WINDOW, "the second pass renewed: {text}");
}

#[tokio::test(start_paused = true)]
async fn an_overlapping_slice_is_novel_only_for_its_unseen_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = numbered_file(dir.path(), 105);
    let turns: Vec<Turn> = vec![
        (None, slice(&path, 0, 35)),  // lines 1-35: new
        (None, slice(&path, 19, 36)), // lines 20-55: 36-55 new
        (None, slice(&path, 10, 20)), // lines 11-30: all seen
        (None, slice(&path, 40, 10)), // lines 41-50: all seen
    ];
    let (result, elapsed) = run_with_tool(
        turns,
        "pw-391-overlap",
        dir.path(),
        "Read",
        Box::new(archon_tools::file_read::ReadTool),
    )
    .await;
    let text = stall_text(result);
    assert!(
        text.contains("last novel activity: turn 2: new tool call Read"),
        "{text}"
    );
    assert!(text.contains("\"offset\":19"), "{text}");
    assert_eq!(elapsed, 2 * 500 + WINDOW, "{text}");
}

#[tokio::test(start_paused = true)]
async fn reading_own_evidence_back_never_renews_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let evidence = dir.path().join("own.jsonl");
    // One 100 000-char line: read-own-evidence returns it in 32 768-byte
    // chunks, so a model pages it with char_offset, each chunk a new slice
    // of a message this session already received.
    let line: String = (0..100_000)
        .map(|index| char::from(b'a' + (index % 26) as u8))
        .collect();
    std::fs::write(&evidence, format!("{line}\n")).unwrap();
    let chunk = |char_offset: usize| {
        Some(serde_json::json!({ "offset": 0, "limit": 1, "char_offset": char_offset }).to_string())
    };
    let turns: Vec<Turn> = [0, 32_768, 65_536, 98_304, 1_000, 16_384]
        .into_iter()
        .map(|char_offset| (None, chunk(char_offset)))
        .collect();
    let (result, elapsed) = run_with_tool(
        turns,
        "pw-391-own-evidence",
        dir.path(),
        "read-own-evidence",
        Box::new(crate::subagent::runner::evidence_recovery::ReadOwnEvidence(
            evidence,
        )),
    )
    .await;
    let text = stall_text(result);
    assert!(!text.contains("new tool call read-own-evidence"), "{text}");
    assert_eq!(elapsed, WINDOW, "no own-evidence slice renewed: {text}");
}

/// Issue 391 negative control: genuinely new slices keep renewing, so a
/// session reading a large file once, slice by slice, is never cut.
#[tokio::test(start_paused = true)]
async fn reading_new_slices_of_one_file_keeps_renewing() {
    let dir = tempfile::tempdir().unwrap();
    let path = numbered_file(dir.path(), 30 * 35);
    let mut turns: Vec<Turn> = (0..30)
        .map(|step| (None, slice(&path, step * 35, 35)))
        .collect();
    turns.push((Some("done"), None));
    let (result, elapsed) = run_with_tool(
        turns,
        "pw-391-new-slices",
        dir.path(),
        "Read",
        Box::new(archon_tools::file_read::ReadTool),
    )
    .await;
    assert_eq!(result.expect("new content is progress"), "done");
    assert!(elapsed >= 15_000, "ran {elapsed}s, past 7200s in total");
}
