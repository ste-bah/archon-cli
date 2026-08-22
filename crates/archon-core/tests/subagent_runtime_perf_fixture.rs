//! Fixed subagent fixture used as the evidence source for issue #171
//! parts 1, 3 and 7.
//!
//! Two things are asserted here, both from *captured outbound request bodies*
//! rather than from unit-level reasoning about the code:
//!
//! 1. Byte stability. Every request the runner hands to the provider is
//!    snapshotted in the exact shape `request_size_breakdown` measures — model,
//!    max_tokens, system, messages, tools, thinking, speed, effort, extra,
//!    request_origin, reasoning_encrypted — and written to the file named by
//!    `ARCHON_171_CAPTURE`. The before/after files must be byte-identical,
//!    `cache_control` marker positions included.
//! 2. Trigger parity. The pressure fixture keeps the shipped
//!    `rate_limit_pressure_*` thresholds so the round at which proactive
//!    compaction fires is part of the captured record.
//!
//! The same fixture doubles as the bench workload: 20 rounds over a transcript
//! that grows past 400KB. `bench_twenty_round_transcript` is `#[ignore]`d so it
//! only runs when asked for, and it runs the fixture in the *digesting* mode
//! (see [`capture`]) because a harness holding all twenty bodies owns the peak
//! working set and hides whatever the runner does with its own memory.

// An integration-test root is a crate root, so a bare `mod` would look for
// `tests/capture.rs` — which cargo would then build as its own test target.
#[path = "subagent_runtime_perf_fixture/capture.rs"]
mod capture;

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use archon_core::agent::AgentConfig;
use archon_core::subagent::runner::SubagentRunner;
use archon_llm::compaction_policy::ProviderFamily;
use archon_llm::identity::{IdentityMode, IdentityProvider};
use archon_llm::provider::{
    LlmError, LlmProvider, LlmRequest, LlmResponse, ModelInfo, ProviderFeature,
};
use archon_llm::streaming::StreamEvent;
use archon_llm::types::{ContentBlockType, Usage};
use archon_tools::tool::{AgentMode, PermissionLevel, Tool, ToolContext, ToolResult};

use capture::{BodyRecord, BodySummary, request_snapshot, snapshot_bytes};

/// Bytes of tool output per round. 20 rounds x 20KB clears the 400KB
/// transcript the issue's bench asks for.
const PAYLOAD_BYTES: usize = 22_000;
const ROUNDS: u32 = 20;

// ---------------------------------------------------------------------------
// Fixture provider: captures every outbound request body.
// ---------------------------------------------------------------------------

struct CaptureProvider {
    rounds: u32,
    calls: AtomicU32,
    compaction_calls: AtomicU32,
    bodies: Mutex<BodyRecord>,
    /// Round index (0-based) of every request that arrived *after* a
    /// compaction summary was requested, so trigger timing is observable.
    compaction_rounds: Mutex<Vec<u32>>,
}

impl CaptureProvider {
    fn new(rounds: u32, bodies: BodyRecord) -> Self {
        Self {
            rounds,
            calls: AtomicU32::new(0),
            compaction_calls: AtomicU32::new(0),
            bodies: Mutex::new(bodies),
            compaction_rounds: Mutex::new(Vec::new()),
        }
    }

    fn snapshots(&self) -> Vec<serde_json::Value> {
        self.bodies.lock().expect("capture lock").snapshots()
    }

    fn summary(&self) -> BodySummary {
        self.bodies.lock().expect("capture lock").summary()
    }

    fn compaction_rounds(&self) -> Vec<u32> {
        self.compaction_rounds.lock().expect("capture lock").clone()
    }
}

#[async_trait::async_trait]
impl LlmProvider for CaptureProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![ModelInfo {
            id: "claude-fixture-4".into(),
            display_name: "Fixture".into(),
            context_window: 1_000_000,
        }]
    }

    fn supports_feature(&self, feature: ProviderFeature) -> bool {
        matches!(
            feature,
            ProviderFeature::Streaming | ProviderFeature::ToolUse
        )
    }

    fn cache_strategy(&self, _model: &str) -> archon_llm::cache_strategy::CacheStrategy {
        archon_llm::cache_strategy::ANTHROPIC_API
    }

    fn compaction_provider_family(&self) -> ProviderFamily {
        ProviderFamily::AnthropicApi
    }

    fn resolve_alias(&self, _alias: &str) -> Option<String> {
        None
    }

    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<tokio::sync::mpsc::Receiver<StreamEvent>, LlmError> {
        if request.request_origin.as_deref() == Some("compaction_summary") {
            self.compaction_calls.fetch_add(1, Ordering::SeqCst);
            self.compaction_rounds
                .lock()
                .expect("capture lock")
                .push(self.calls.load(Ordering::SeqCst));
            return Ok(stream_of(vec![
                StreamEvent::TextDelta {
                    index: 0,
                    text: "Fixture compaction summary.".into(),
                },
                StreamEvent::MessageStop,
            ])
            .await);
        }

        self.bodies
            .lock()
            .expect("capture lock")
            .record(request_snapshot(&request));
        let round = self.calls.fetch_add(1, Ordering::SeqCst);

        let events = if round + 1 < self.rounds {
            bulk_tool_turn(round)
        } else {
            text_turn("fixture complete")
        };
        Ok(stream_of(events).await)
    }

    async fn complete(&self, _request: LlmRequest) -> Result<LlmResponse, LlmError> {
        unreachable!("the fixture only streams")
    }
}

async fn stream_of(events: Vec<StreamEvent>) -> tokio::sync::mpsc::Receiver<StreamEvent> {
    let (tx, rx) = tokio::sync::mpsc::channel(events.len() + 1);
    for event in events {
        tx.send(event).await.expect("fixture stream send");
    }
    rx
}

fn bulk_tool_turn(round: u32) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: format!("fixture-{round}"),
            model: "claude-fixture-4".into(),
            usage: Usage::default(),
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
            tool_use_id: None,
            tool_name: None,
        },
        StreamEvent::TextDelta {
            index: 0,
            text: format!("reading chunk {round}"),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::ContentBlockStart {
            index: 1,
            block_type: ContentBlockType::ToolUse,
            tool_use_id: Some(format!("tool-{round}")),
            tool_name: Some("Bulk".into()),
        },
        StreamEvent::InputJsonDelta {
            index: 1,
            partial_json: serde_json::json!({ "seed": round }).to_string(),
        },
        StreamEvent::ContentBlockStop { index: 1 },
        StreamEvent::MessageStop,
    ]
}

fn text_turn(text: &str) -> Vec<StreamEvent> {
    vec![
        StreamEvent::MessageStart {
            id: "fixture-final".into(),
            model: "claude-fixture-4".into(),
            usage: Usage::default(),
        },
        StreamEvent::ContentBlockStart {
            index: 0,
            block_type: ContentBlockType::Text,
            tool_use_id: None,
            tool_name: None,
        },
        StreamEvent::TextDelta {
            index: 0,
            text: text.into(),
        },
        StreamEvent::ContentBlockStop { index: 0 },
        StreamEvent::MessageStop,
    ]
}

// ---------------------------------------------------------------------------
// Fixture tool: deterministic, sized output.
// ---------------------------------------------------------------------------

struct BulkTool;

#[async_trait::async_trait]
impl Tool for BulkTool {
    fn name(&self) -> &str {
        "Bulk"
    }

    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::HostLocal
    }

    fn description(&self) -> &str {
        "Returns a deterministic block of text sized for the #171 fixture."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": { "seed": { "type": "integer" } },
            "required": ["seed"]
        })
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let seed = input.get("seed").and_then(|v| v.as_u64()).unwrap_or(0);
        let unit = format!("chunk-{seed:04}-");
        let mut out = String::with_capacity(PAYLOAD_BYTES + unit.len());
        while out.len() < PAYLOAD_BYTES {
            out.push_str(&unit);
        }
        ToolResult::success(out)
    }

    fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
        PermissionLevel::Safe
    }
}

// ---------------------------------------------------------------------------
// Fixture wiring.
// ---------------------------------------------------------------------------

fn stable_system_blocks() -> Vec<serde_json::Value> {
    (0..3)
        .map(|index| {
            serde_json::json!({
                "type": "text",
                "text": format!("stable workflow universe block {index}: {}", "u".repeat(2_000)),
            })
        })
        .collect()
}

fn fixture_config(pressure: bool) -> AgentConfig {
    let mut config = AgentConfig {
        session_id: "issue-171-fixture".into(),
        ..AgentConfig::default()
    };
    config.context.context_window_override = Some(2_000_000);
    if !pressure {
        config.context.rate_limit_pressure_tokens = None;
        config.context.rate_limit_pressure_body_bytes = None;
    }
    config
}

fn build_runner(provider: Arc<CaptureProvider>, pressure: bool) -> SubagentRunner {
    let mut registry = archon_core::dispatch::create_default_registry(std::env::temp_dir(), None);
    registry.register(Box::new(BulkTool));
    let registry = Arc::new(registry);
    let tool_defs = registry.tool_definitions();

    let mut runner = SubagentRunner::new(
        provider,
        "You are the issue-171 perf fixture subagent.".into(),
        tool_defs,
        registry,
        ToolContext {
            working_dir: std::env::temp_dir(),
            session_id: "issue-171-fixture".into(),
            mode: AgentMode::Normal,
            // Off for this fixture, and it has to be. Three tests in this
            // binary run the fixture concurrently, two of them run it twice,
            // and every pass claims the same agent identity — so the
            // repeat-tool chain (#200 Phase 2), which is keyed per agent,
            // would see unrelated passes as one agent and could put an
            // advisory into the very request bodies these tests assert are
            // byte-identical. The session id is itself part of those bytes, so
            // giving each pass its own identity is not available either. This
            // fixture measures what the runner serializes, not what the guard
            // advises; the guard has its own tests.
            repeat_tool: archon_tools::repeat_tool_guard::RepeatToolConfig {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        },
        "claude-fixture-4".into(),
        ROUNDS + 4,
        600,
        Arc::new(fixture_config(pressure)),
        Arc::new(IdentityProvider::new(
            IdentityMode::Clean,
            "issue-171-fixture".into(),
            String::new(),
            String::new(),
        )),
    );
    runner.set_request_system(stable_system_blocks());
    runner.set_critical_system_reminder("stay on the fixture path".into());
    runner
}

/// Run one fixture pass with the given body bookkeeping.
async fn run_fixture_with(pressure: bool, bodies: BodyRecord) -> Arc<CaptureProvider> {
    let provider = Arc::new(CaptureProvider::new(ROUNDS, bodies));
    let runner = build_runner(provider.clone(), pressure);
    let output = runner
        .run("run the issue-171 fixture")
        .await
        .expect("fixture subagent must finish");
    assert_eq!(output, "fixture complete");
    provider
}

/// Run one fixture pass and hand back every captured outbound body.
async fn run_fixture(pressure: bool) -> (Vec<serde_json::Value>, Vec<u32>) {
    let provider = run_fixture_with(pressure, BodyRecord::retaining()).await;
    (provider.snapshots(), provider.compaction_rounds())
}

/// Run one fixture pass keeping only the digest of what was sent.
///
/// This is the arm to measure memory in: nothing the fixture itself allocates
/// outlives the round that produced it, so the process high-water mark belongs
/// to the runner rather than to the harness.
async fn run_fixture_digested(pressure: bool) -> BodySummary {
    run_fixture_with(pressure, BodyRecord::digesting())
        .await
        .summary()
}

fn transcript_bytes(snapshot: &serde_json::Value) -> usize {
    serde_json::to_vec(&snapshot["messages"])
        .expect("serialize fixture messages")
        .len()
}

fn write_capture(name: &str, snapshots: &[serde_json::Value], compaction_rounds: &[u32]) {
    let Ok(dir) = std::env::var("ARCHON_171_CAPTURE") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("create capture dir");
    let mut out: Vec<u8> = Vec::new();
    for snapshot in snapshots {
        out.extend_from_slice(&snapshot_bytes(snapshot));
    }
    out.extend_from_slice(format!("compaction_rounds={compaction_rounds:?}\n").as_bytes());
    std::fs::write(dir.join(format!("{name}.jsonl")), out).expect("write capture");
}

// ---------------------------------------------------------------------------
// Evidence tests.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capture_quiet_fixture_request_bodies() {
    let (snapshots, compaction_rounds) = run_fixture(false).await;

    assert_eq!(snapshots.len(), ROUNDS as usize);
    assert!(
        compaction_rounds.is_empty(),
        "the quiet fixture must not compact: {compaction_rounds:?}"
    );
    assert!(
        transcript_bytes(snapshots.last().expect("final snapshot")) > 400_000,
        "fixture transcript must exceed 400KB, got {}",
        transcript_bytes(snapshots.last().expect("final snapshot"))
    );
    // Anthropic cache markers must be present and position-sensitive: the last
    // system block carries one, and the newest cacheable content block does.
    let last = snapshots.last().expect("final snapshot");
    let system = last["system"].as_array().expect("system array");
    assert_eq!(
        system[system.len() - 1]["cache_control"]["type"],
        "ephemeral",
        "system cache marker must sit on the final system block"
    );
    for block in &system[..system.len() - 1] {
        assert!(
            block.get("cache_control").is_none(),
            "only the final system block may carry a marker: {block}"
        );
    }
    let messages = last["messages"].as_array().expect("messages array");
    let marked: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.to_string().contains("cache_control"))
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        marked,
        vec![messages.len() - 1],
        "exactly one conversation marker, on the newest message"
    );

    write_capture("quiet", &snapshots, &compaction_rounds);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capture_pressure_fixture_request_bodies() {
    let (snapshots, compaction_rounds) = run_fixture(true).await;

    assert!(
        !compaction_rounds.is_empty(),
        "the pressure fixture must reach the request-pressure threshold"
    );
    write_capture("pressure", &snapshots, &compaction_rounds);
    println!("pressure fixture compacted at rounds {compaction_rounds:?}");
}

/// The memory arm has to be the same program as the byte-identity arm.
///
/// Dropping each body after folding it in is only a legitimate way to measure
/// peak RSS if the bodies are still built and serialized identically — an arm
/// that skipped work would be measuring a different program. Running both
/// modes over the same fixture and comparing the digest is what makes the
/// non-retaining mode usable as a standing gate: the number it prints is the
/// capture file's body bytes, so it moves exactly when the capture would.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn digesting_and_retaining_fixtures_send_the_same_bytes() {
    let digested = run_fixture_digested(false).await;
    let (snapshots, _) = run_fixture(false).await;
    let retained = BodyRecord::Retained(snapshots).summary();

    assert_eq!(digested, retained);
    assert_eq!(digested.bodies, ROUNDS);
    println!("quiet fixture bodies: {digested}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "bench fixture; run explicitly for #171 numbers"]
async fn bench_twenty_round_transcript() {
    let iterations: u32 = std::env::var("ARCHON_171_ITERATIONS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5);

    let mut millis: Vec<u128> = Vec::new();
    let mut summary = None;
    for _ in 0..iterations {
        let started = std::time::Instant::now();
        let sent = run_fixture_digested(false).await;
        millis.push(started.elapsed().as_millis());
        assert_eq!(sent.bodies, ROUNDS);
        summary = Some(sent);
    }
    let total: u128 = millis.iter().sum();
    println!(
        "bench_twenty_round_transcript iterations={iterations} per_run_ms={millis:?} mean_ms={} sent={}",
        total / u128::from(iterations),
        summary.expect("at least one iteration"),
    );
}
