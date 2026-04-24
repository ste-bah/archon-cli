//! TASK-AGS-OBS-915: 100-subagent spawn latency test.
//!
//! Adapted scope: measures per-spawn latency for 100 stub subagents driven
//! through the real `AgentTool::execute` path in `archon-tools`.
//!
//! The stale spec referenced `archon-discovery` (non-existent). The actual
//! spawn entrypoint is `crates/archon-tools/src/agent_tool.rs::AgentTool::execute`
//! which `tokio::spawn`s the runner via `run_subagent` (agent_tool.rs:315
//! and :522). We exercise that path directly using the established
//! `StubExecutor` pattern.
//!
//! Pass criterion: p95 per-spawn latency < 50ms on a warm cache.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
use std::time::{Duration, Instant};

use serde_json::json;

use archon_tools::agent_tool::{AgentTool, SubagentRequest};
use archon_tools::cancel_background_agent;
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor,
    install_subagent_executor,
};
use archon_tools::tool::{AgentMode, Tool, ToolContext};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Stub executor — no-op runner so each spawn measures the registry+spawn
// path only (no LLM I/O). Same pattern as task_ags_104.rs.
// ---------------------------------------------------------------------------

struct StubExecutor;

#[async_trait]
impl SubagentExecutor for StubExecutor {
    async fn run_to_completion(
        &self,
        _subagent_id: String,
        _request: SubagentRequest,
        _ctx: ToolContext,
        _cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        Ok(String::new())
    }

    async fn on_inner_complete(&self, _subagent_id: String, _result: Result<String, String>) {}

    async fn on_visible_complete(
        &self,
        _subagent_id: String,
        _result: Result<String, String>,
        _nested: bool,
    ) -> OutcomeSideEffects {
        OutcomeSideEffects::default()
    }

    fn auto_background_ms(&self) -> u64 {
        0
    }

    fn classify(&self, req: &SubagentRequest) -> SubagentClassification {
        if req.run_in_background {
            SubagentClassification::ExplicitBackground
        } else {
            SubagentClassification::Foreground
        }
    }
}

static INSTALL_ONCE: Once = Once::new();

fn ensure_stub_executor() {
    INSTALL_ONCE.call_once(|| {
        install_subagent_executor(Arc::new(StubExecutor));
    });
}

fn make_ctx() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("/tmp"),
        session_id: "task-obs-915-test".into(),
        mode: AgentMode::Normal,
        extra_dirs: vec![],
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Quantile helper — nearest-rank on a sorted slice.
// ---------------------------------------------------------------------------

fn quantile_us(sorted: &[Duration], q: f64) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let n = sorted.len();
    let idx = ((q * n as f64).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1);
    sorted[idx].as_micros()
}

// ---------------------------------------------------------------------------
// 100 stub spawns, p95 latency assertion.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn obs_915_100_stub_spawns_p95_latency() {
    ensure_stub_executor();
    let tool = Arc::new(AgentTool::new());

    // Pre-warm: one execute call to page in DashMap + serde_json + OnceLock.
    {
        let warm = tool
            .execute(
                json!({ "prompt": "obs-915 pre-warm", "run_in_background": true }),
                &make_ctx(),
            )
            .await;
        assert!(!warm.is_error, "pre-warm failed: {}", warm.content);
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&warm.content) {
            if let Some(id_str) = v["agent_id"].as_str() {
                if let Ok(id) = Uuid::parse_str(id_str) {
                    let _ = cancel_background_agent(&id);
                }
            }
        }
    }

    let samples: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::with_capacity(100)));
    let mut spawned_ids: Vec<Uuid> = Vec::with_capacity(100);

    // Spawn 100 sequential stub subagents and measure each.
    for i in 0..100 {
        let input = json!({
            "prompt": format!("obs-915 probe {i}"),
            "run_in_background": true,
        });
        let ctx = make_ctx();

        let t0 = Instant::now();
        let result = tool.execute(input, &ctx).await;
        let elapsed = t0.elapsed();

        assert!(!result.is_error, "spawn {i} failed: {}", result.content);
        let v: serde_json::Value =
            serde_json::from_str(&result.content).expect("spawn result must be JSON");
        assert_eq!(v["status"], "spawned", "spawn {i} status mismatch");

        let id = Uuid::parse_str(v["agent_id"].as_str().expect("agent_id missing"))
            .expect("agent_id must be UUID");
        spawned_ids.push(id);
        samples.lock().unwrap().push(elapsed);
    }

    let mut sorted: Vec<Duration> = samples.lock().unwrap().clone();
    sorted.sort();

    let p50 = quantile_us(&sorted, 0.50);
    let p95 = quantile_us(&sorted, 0.95);
    let p99 = quantile_us(&sorted, 0.99);
    let max = sorted.last().copied().unwrap_or_default();
    let avg_us: u128 = sorted.iter().map(|d| d.as_micros()).sum::<u128>() / sorted.len() as u128;

    println!(
        "OBS-915: 100 spawns — avg={}us p50={}us p95={}us p99={}us max={:?}",
        avg_us, p50, p95, p99, max
    );

    // Budget: p95 < 50ms (generous; observed on dev machine ~7-15us).
    assert!(
        p95 < 50_000,
        "p95 spawn latency {}us exceeded 50ms budget",
        p95
    );

    // Cleanup.
    for id in &spawned_ids {
        let _ = cancel_background_agent(id);
    }
}
