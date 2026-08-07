//! Issue #129: every spawn path is registered by the one function they all
//! funnel through, so agent liveness is a single lookup.
//!
//! The unit tests in `board/leases_tests.rs` register by hand, which proves the
//! lease reads the registry correctly but not that the runners write to it.
//! This file drives the public runners with a stub executor and asks
//! `holder_liveness` the same question the lease sweep asks.
//!
//! `archon-pipeline` is the crate that exposed the defect, and it is also the
//! one that cannot be called from here (it depends on `archon-tools`, not the
//! other way round). What it does is call `run_subagent_with_system` /
//! `run_subagent_foreground_with_system` with a `{session}-{ordinal}-{agent}`
//! subagent id, which is exactly what these tests do.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use archon_tools::agent_tool::{
    SubagentRequest, run_subagent_foreground_with_system, run_subagent_with_system,
};
use archon_tools::board::{HolderLiveness, holder_liveness};
use archon_tools::subagent_executor::{
    ExecutorError, OutcomeSideEffects, SubagentClassification, SubagentExecutor, SubagentOutcome,
    install_subagent_executor,
};
use archon_tools::tool::{AgentMode, ToolContext};
use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

/// Auto-background timer for the installed stub. Long enough that a run which
/// completes promptly is not racing it, short enough that the auto-background
/// test does not sit around waiting.
const AUTO_BACKGROUND_MS: u64 = 100;

/// A stub whose runs finish only when the test releases that particular agent.
/// The gate is keyed by subagent id rather than being a single flag because
/// `install_subagent_executor` is process-global — one executor for the whole
/// binary — and the tests using it run concurrently.
#[derive(Clone, Debug, Default)]
struct Gate {
    released: Arc<Mutex<HashSet<String>>>,
}

impl Gate {
    fn release(&self, subagent_id: &str) {
        self.released
            .lock()
            .expect("gate poisoned")
            .insert(subagent_id.to_string());
    }

    /// Close the gate again so the same id can be run a second time — which is
    /// what `SendMessage` does on resume.
    fn hold(&self, subagent_id: &str) {
        self.released
            .lock()
            .expect("gate poisoned")
            .remove(subagent_id);
    }

    fn is_released(&self, subagent_id: &str) -> bool {
        self.released
            .lock()
            .expect("gate poisoned")
            .contains(subagent_id)
    }
}

struct GatedExecutor {
    gate: Gate,
}

#[async_trait]
impl SubagentExecutor for GatedExecutor {
    async fn run_to_completion(
        &self,
        subagent_id: String,
        _request: SubagentRequest,
        _ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        loop {
            if self.gate.is_released(&subagent_id) {
                return Ok("done".to_string());
            }
            tokio::select! {
                _ = cancel.cancelled() => {
                    return Err(ExecutorError::Internal("subagent cancelled".to_string()));
                }
                _ = tokio::time::sleep(Duration::from_millis(5)) => {}
            }
        }
    }

    async fn run_to_completion_with_system(
        &self,
        subagent_id: String,
        request: SubagentRequest,
        _system: Vec<serde_json::Value>,
        ctx: ToolContext,
        cancel: CancellationToken,
    ) -> Result<String, ExecutorError> {
        // Unlike the default impl, accept a system block: the pipeline always
        // sends one, and rejecting it would fail the run before it ever
        // registered.
        self.run_to_completion(subagent_id, request, ctx, cancel)
            .await
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
        AUTO_BACKGROUND_MS
    }

    fn classify(&self, _request: &SubagentRequest) -> SubagentClassification {
        SubagentClassification::Foreground
    }
}

static INSTALL_ONCE: Once = Once::new();
static GATE: std::sync::OnceLock<Gate> = std::sync::OnceLock::new();

/// Install the stub once and hand back the gate that releases its runs.
fn gate() -> &'static Gate {
    INSTALL_ONCE.call_once(|| {
        let gate = Gate::default();
        GATE.set(gate.clone()).expect("gate set once");
        install_subagent_executor(Arc::new(GatedExecutor { gate }));
    });
    GATE.get().expect("gate installed")
}

fn pipeline_request() -> SubagentRequest {
    SubagentRequest {
        prompt: "review the diff".to_string(),
        model: None,
        allowed_tools: vec!["Read".to_string()],
        max_turns: SubagentRequest::DEFAULT_MAX_TURNS,
        timeout_secs: SubagentRequest::DEFAULT_TIMEOUT_SECS,
        subagent_type: Some("implementer".to_string()),
        run_in_background: false,
        cwd: None,
        isolation: None,
        provider_env: None,
    }
}

fn ctx() -> ToolContext {
    ToolContext {
        working_dir: PathBuf::from("."),
        session_id: "subagent-liveness-choke-point".into(),
        mode: AgentMode::Normal,
        ..Default::default()
    }
}

/// Poll until `holder_liveness` reaches `want`, or give up. Liveness is written
/// from the runner's task, so the test has to let that task run rather than
/// assume an ordering.
async fn await_liveness(agent: &str, want: HolderLiveness) {
    for _ in 0..200 {
        if holder_liveness(agent) == want {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "{agent} never reached {want}; it is {}",
        holder_liveness(agent)
    );
}

/// The defect itself: a pipeline-spawned subagent used to be in no registry,
/// so it read as dead from birth and the lease sweep released its claims while
/// it was still working.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pipeline_subagent_is_live_while_it_runs_and_dead_once_it_stops() {
    let gate = gate();
    let agent = "sess-choke-1-implementer".to_string();

    let run = tokio::spawn(run_subagent_foreground_with_system(
        agent.clone(),
        pipeline_request(),
        vec![serde_json::json!({"type": "text", "text": "system"})],
        CancellationToken::new(),
        ctx(),
    ));

    await_liveness(&agent, HolderLiveness::Live).await;

    gate.release(&agent);
    let outcome = run.await.expect("runner must not panic");
    assert!(
        matches!(outcome, SubagentOutcome::Completed(_)),
        "unexpected outcome: {outcome:?}"
    );

    assert_eq!(
        holder_liveness(&agent),
        HolderLiveness::Dead,
        "a subagent that has returned must not still hold its claims"
    );
}

/// The auto-background arm is where a hook-based release would leak: the runner
/// keeps executing after `run_subagent` has returned `AutoBackgrounded`, and
/// `SubagentStop` never fires for it. The registration has to outlive the
/// return and end with the runner instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_auto_backgrounded_subagent_stays_live_until_its_runner_ends() {
    let gate = gate();
    let agent = "sess-choke-2-reviewer".to_string();

    let outcome = run_subagent_with_system(
        agent.clone(),
        pipeline_request(),
        vec![serde_json::json!({"type": "text", "text": "system"})],
        CancellationToken::new(),
        ctx(),
    )
    .await;

    assert!(
        matches!(outcome, SubagentOutcome::AutoBackgrounded),
        "the gated stub cannot finish before the timer: {outcome:?}"
    );
    assert_eq!(
        holder_liveness(&agent),
        HolderLiveness::Live,
        "the runner is still executing; releasing its claims here is the bug"
    );

    gate.release(&agent);
    await_liveness(&agent, HolderLiveness::Dead).await;
}

/// `SendMessage` resumes an agent under its ORIGINAL subagent id
/// (`archon-core/src/agent/message_delivery.rs`), so the choke point is handed
/// an id it has seen before. Whether the previous entry is still there depends
/// on whether the 60s reaper has been past — a race — so both branches are
/// exercised here and both must end with the resumed agent reported live.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resumed_subagent_is_live_again_whether_or_not_its_last_run_was_reaped() {
    use archon_tools::background_agents::{BACKGROUND_AGENTS, PollOutcome, poll_subagent};

    let gate = gate();
    let agent = "sess-choke-4-implementer".to_string();

    // First life.
    run_once(gate, &agent).await;
    assert_eq!(holder_liveness(&agent), HolderLiveness::Dead);

    // Resume with the terminal entry still in the registry. Inheriting that
    // status is the failure this branch is here to catch.
    assert!(
        matches!(poll_subagent(&agent), PollOutcome::Complete(_)),
        "the premise of this branch is a terminal entry the reaper has not taken"
    );
    run_once(gate, &agent).await;

    // Resume after the reaper has taken it: an ordinary fresh registration.
    BACKGROUND_AGENTS.reap_finished();
    assert_eq!(
        poll_subagent(&agent),
        PollOutcome::Unknown,
        "the premise of this branch is that the entry is gone"
    );
    run_once(gate, &agent).await;
    assert_eq!(holder_liveness(&agent), HolderLiveness::Dead);
}

/// Run `agent` to completion once, asserting it is live in between. Shared by
/// the resume test's three lives.
async fn run_once(gate: &Gate, agent: &str) {
    gate.hold(agent);
    let run = tokio::spawn(run_subagent_foreground_with_system(
        agent.to_string(),
        pipeline_request(),
        vec![serde_json::json!({"type": "text", "text": "system"})],
        CancellationToken::new(),
        ctx(),
    ));
    await_liveness(agent, HolderLiveness::Live).await;
    gate.release(agent);
    let outcome = run.await.expect("runner must not panic");
    assert!(
        matches!(outcome, SubagentOutcome::Completed(_)),
        "unexpected outcome: {outcome:?}"
    );
}

/// Cancellation is the third way a runner ends, and the abort path in
/// `await_cancelled_foreground` never reaches the runner's own tail.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_subagent_stops_being_live() {
    gate();
    let agent = "sess-choke-3-implementer".to_string();
    let cancel = CancellationToken::new();

    let run = tokio::spawn(run_subagent_foreground_with_system(
        agent.clone(),
        pipeline_request(),
        vec![serde_json::json!({"type": "text", "text": "system"})],
        cancel.clone(),
        ctx(),
    ));

    await_liveness(&agent, HolderLiveness::Live).await;
    cancel.cancel();
    let outcome = run.await.expect("runner must not panic");
    assert!(
        matches!(outcome, SubagentOutcome::Cancelled),
        "unexpected outcome: {outcome:?}"
    );

    assert_eq!(holder_liveness(&agent), HolderLiveness::Dead);
}
