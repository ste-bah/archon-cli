//! The runner enforces the `NextAgent::ContinueWave` contract, never lets a
//! step complete without appending exactly its own results, and bounds
//! consecutive skips. Audited runs record each violation as a failed bundle.
//!
//! Each run is wrapped in a short timeout, and the facade yields on every
//! `next_agent` call, so a regression to the old silent-drop loop fails fast
//! instead of hanging the test binary.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use std::path::Path;

use archon_pipeline::audit::{BundleStatus, PipelineBundleStore};
use archon_pipeline::runner::{
    AgentInfo, AgentResult, LlmClient, LlmResponse, MAX_CONSECUTIVE_SKIPS, NextAgent,
    PARALLEL_WAVE_LIMIT, PipelineFacade, PipelineResult, PipelineSession, PipelineType,
    QualityScore, ToolAccessLevel, run_pipeline, run_pipeline_audited,
};

const RUN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_NEXT_AGENT_CALLS: usize = 3 * MAX_CONSECUTIVE_SKIPS;
const SESSION_ID: &str = "wave-contract";

#[derive(Clone, Default)]
struct CountingLlm {
    calls: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl LlmClient for CountingLlm {
    async fn send_message(
        &self,
        _messages: Vec<serde_json::Value>,
        _system: Vec<serde_json::Value>,
        _tools: Vec<serde_json::Value>,
        _model: &str,
    ) -> anyhow::Result<LlmResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            content: "ok".to_string(),
            tool_uses: Vec::new(),
            tokens_in: 1,
            tokens_out: 1,
            stop_reason: Some("end_turn".to_string()),
        })
    }
}

fn agent(index: usize, parallelizable: bool) -> AgentInfo {
    AgentInfo {
        key: format!("agent-{index}"),
        display_name: format!("Agent {index}"),
        model: "test-model".to_string(),
        phase: 1,
        critical: true,
        parallelizable,
        quality_threshold: 0.5,
        tool_access_level: ToolAccessLevel::ReadOnly,
    }
}

/// What `process_completion` does to the runner-owned result list.
#[derive(Clone, Copy)]
enum Mutation {
    None,
    /// Clear the results once they hold at least this many entries.
    ClearAtLeast(usize),
    /// Rename the key of the last recorded result.
    RenameLast,
}

/// Emits `step(call)` (1-based call number) until the session holds `target`
/// results.
struct StepFacade {
    step: fn(usize) -> NextAgent,
    target: usize,
    mutation: Mutation,
    next_agent_calls: AtomicUsize,
}

impl StepFacade {
    fn new(step: fn(usize) -> NextAgent, target: usize) -> Self {
        Self {
            step,
            target,
            mutation: Mutation::None,
            next_agent_calls: AtomicUsize::new(0),
        }
    }

    fn with_mutation(mut self, mutation: Mutation) -> Self {
        self.mutation = mutation;
        self
    }
}

#[async_trait::async_trait]
impl PipelineFacade for StepFacade {
    async fn init_session(&self, task: &str) -> anyhow::Result<PipelineSession> {
        Ok(PipelineSession {
            id: SESSION_ID.to_string(),
            pipeline_type: PipelineType::Coding,
            task: task.to_string(),
            started_at: Instant::now(),
            agent_results: Vec::new(),
            leann_context: String::new(),
        })
    }

    async fn next_agent(&self, session: &PipelineSession) -> anyhow::Result<NextAgent> {
        // Yield so the timeout can fire if the runner ever loops again.
        tokio::task::yield_now().await;
        let calls = self.next_agent_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if calls > MAX_NEXT_AGENT_CALLS {
            anyhow::bail!("runner looped: next_agent called {calls} times");
        }
        if session.agent_results.len() >= self.target {
            return Ok(NextAgent::Done);
        }
        Ok((self.step)(calls))
    }

    async fn build_prompt(
        &self,
        _session: &PipelineSession,
        agent: &AgentInfo,
    ) -> anyhow::Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
    )> {
        Ok((
            vec![serde_json::json!({"role": "user", "content": agent.key})],
            Vec::new(),
            Vec::new(),
        ))
    }

    async fn score_quality(
        &self,
        _session: &PipelineSession,
        _agent: &AgentInfo,
        _result: &AgentResult,
    ) -> anyhow::Result<QualityScore> {
        Ok(QualityScore {
            overall: 0.9,
            dimensions: HashMap::new(),
        })
    }

    async fn process_completion(
        &self,
        session: &mut PipelineSession,
        _agent: &AgentInfo,
        _result: &AgentResult,
        _quality: &QualityScore,
    ) -> anyhow::Result<()> {
        match self.mutation {
            Mutation::None => {}
            Mutation::ClearAtLeast(len) if session.agent_results.len() >= len => {
                session.agent_results.clear();
            }
            Mutation::ClearAtLeast(_) => {}
            Mutation::RenameLast => {
                if let Some((agent, _)) = session.agent_results.last_mut() {
                    agent.key = "intruder".to_string();
                }
            }
        }
        Ok(())
    }

    async fn finalize(&self, session: PipelineSession) -> anyhow::Result<PipelineResult> {
        Ok(PipelineResult {
            session_id: session.id.clone(),
            pipeline_type: session.pipeline_type.clone(),
            total_cost_usd: 0.0,
            duration: session.started_at.elapsed(),
            final_output: "done".to_string(),
            agent_results: session.agent_results,
        })
    }
}

async fn run_bounded(facade: &StepFacade, llm: &CountingLlm) -> anyhow::Result<PipelineResult> {
    tokio::time::timeout(
        RUN_TIMEOUT,
        run_pipeline(facade, llm, "wave contract", None, None, None),
    )
    .await
    .expect("runner must return within the timeout, not loop")
}

async fn run_audited_bounded(
    facade: &StepFacade,
    llm: &CountingLlm,
    worktree: &Path,
) -> anyhow::Result<PipelineResult> {
    tokio::time::timeout(
        RUN_TIMEOUT,
        run_pipeline_audited(facade, llm, "wave contract", worktree, None, None, None),
    )
    .await
    .expect("audited runner must return within the timeout, not loop")
}

/// Read the bundle back from disk: state and event log must both record the
/// failure with the error the run returned.
fn assert_bundle_failed(worktree: &Path, message: &str) {
    let store = PipelineBundleStore::new(worktree);
    let state = store.load_state(SESSION_ID).expect("bundle state loads");
    assert_eq!(state.status, BundleStatus::Failed);
    assert_eq!(state.last_error.as_deref(), Some(message));
    let log = std::fs::read_to_string(store.bundle_dir(SESSION_ID).join("audit.log"))
        .expect("audit log reads");
    let failures: Vec<String> = log
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event line parses"))
        .filter(|event| event["type"] == "run_failed")
        .map(|event| event["error"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(failures, vec![message.to_string()], "{log}");
}

fn expect_err(result: anyhow::Result<PipelineResult>) -> String {
    match result {
        Ok(_) => panic!("the run must fail"),
        Err(error) => error.to_string(),
    }
}

#[tokio::test]
async fn wave_of_non_parallelizable_agents_is_a_contract_error() {
    let facade = StepFacade::new(
        |_| NextAgent::ContinueWave(vec![agent(0, false), agent(1, false)]),
        2,
    );
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(message.contains("contract violation"), "{message}");
    assert!(message.contains("not parallelizable"), "{message}");
    assert!(message.contains("agent-0, agent-1"), "{message}");
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "no member may run");
}

#[tokio::test]
async fn wave_with_one_serial_member_names_only_that_member() {
    let facade = StepFacade::new(
        |_| NextAgent::ContinueWave(vec![agent(0, true), agent(1, false)]),
        2,
    );
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(
        message.contains("agents [agent-1] are not parallelizable"),
        "{message}"
    );
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "no member may run");
}

#[tokio::test]
async fn wave_above_the_limit_is_a_contract_error() {
    let facade = StepFacade::new(
        |_| NextAgent::ContinueWave((0..=PARALLEL_WAVE_LIMIT).map(|i| agent(i, true)).collect()),
        PARALLEL_WAVE_LIMIT + 1,
    );
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(message.contains("contract violation"), "{message}");
    assert!(
        message.contains(&format!("the wave has {} agents", PARALLEL_WAVE_LIMIT + 1)),
        "{message}"
    );
    assert!(
        message.contains(&format!("agent-{PARALLEL_WAVE_LIMIT}")),
        "the excess agent must be named, not dropped: {message}"
    );
    assert_eq!(llm.calls.load(Ordering::SeqCst), 0, "no member may run");
}

#[tokio::test]
async fn empty_wave_is_a_contract_error() {
    let facade = StepFacade::new(|_| NextAgent::ContinueWave(Vec::new()), 1);
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(message.contains("contract violation"), "{message}");
    assert!(message.contains("the wave is empty"), "{message}");
}

#[tokio::test]
async fn wave_at_the_limit_runs_every_member() {
    let facade = StepFacade::new(
        |_| NextAgent::ContinueWave((0..PARALLEL_WAVE_LIMIT).map(|i| agent(i, true)).collect()),
        PARALLEL_WAVE_LIMIT,
    );
    let llm = CountingLlm::default();

    let result = run_bounded(&facade, &llm)
        .await
        .expect("wave at limit runs");

    let keys: Vec<String> = result
        .agent_results
        .iter()
        .map(|(info, _)| info.key.clone())
        .collect();
    let expected: Vec<String> = (0..PARALLEL_WAVE_LIMIT)
        .map(|i| format!("agent-{i}"))
        .collect();
    assert_eq!(keys, expected);
    assert_eq!(llm.calls.load(Ordering::SeqCst), PARALLEL_WAVE_LIMIT);
}

#[tokio::test]
async fn step_that_records_no_result_fails_instead_of_looping() {
    let facade = StepFacade::new(|_| NextAgent::Continue(agent(0, false)), 2)
        .with_mutation(Mutation::ClearAtLeast(1));
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(
        message.contains(
            "progress violation: NextAgent::Continue expected to append [agent-0] \
             after 1 prior result(s), but appended []"
        ),
        "{message}"
    );
}

#[tokio::test]
async fn wave_that_loses_a_result_names_expected_and_actual_keys() {
    let facade = StepFacade::new(
        |_| NextAgent::ContinueWave(vec![agent(0, true), agent(1, true)]),
        2,
    )
    .with_mutation(Mutation::ClearAtLeast(1));
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(
        message.contains(
            "NextAgent::ContinueWave expected to append [agent-0, agent-1] \
             after 0 prior result(s), but appended [agent-1]"
        ),
        "{message}"
    );
}

#[tokio::test]
async fn wave_with_the_right_count_but_wrong_keys_fails() {
    let facade = StepFacade::new(
        |_| NextAgent::ContinueWave(vec![agent(0, true), agent(1, true)]),
        2,
    )
    .with_mutation(Mutation::RenameLast);
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(
        message.contains("expected to append [agent-0, agent-1] after 0 prior result(s), but appended [intruder, agent-1]"),
        "{message}"
    );
}

#[tokio::test]
async fn step_that_shrinks_prior_results_fails() {
    let facade = StepFacade::new(|call| NextAgent::Continue(agent(call - 1, false)), 3)
        .with_mutation(Mutation::ClearAtLeast(2));
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(
        message.contains("expected to append [agent-2] after 2 prior result(s), but the 2 prior result(s) shrank to 1"),
        "{message}"
    );
}

#[tokio::test]
async fn endless_skips_fail_at_the_limit_instead_of_looping() {
    let facade = StepFacade::new(|_| NextAgent::Skip("nothing to do".to_string()), 1);
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(
        message.contains(&format!(
            "{} consecutive NextAgent::Skip with no recorded result \
             (limit MAX_CONSECUTIVE_SKIPS = {MAX_CONSECUTIVE_SKIPS}); last reason: nothing to do",
            MAX_CONSECUTIVE_SKIPS + 1
        )),
        "{message}"
    );
    assert_eq!(
        facade.next_agent_calls.load(Ordering::SeqCst),
        MAX_CONSECUTIVE_SKIPS + 1
    );
}

#[tokio::test]
async fn recorded_result_resets_the_skip_count() {
    // MAX skips, one agent, MAX skips, one agent: never MAX + 1 in a row.
    let facade = StepFacade::new(
        |call| {
            if call % (MAX_CONSECUTIVE_SKIPS + 1) == 0 {
                NextAgent::Continue(agent(call / (MAX_CONSECUTIVE_SKIPS + 1) - 1, false))
            } else {
                NextAgent::Skip("not yet".to_string())
            }
        },
        2,
    );
    let llm = CountingLlm::default();

    let result = run_bounded(&facade, &llm).await.expect("skips reset");

    assert_eq!(result.agent_results.len(), 2);
}

#[tokio::test]
async fn audited_run_records_a_wave_contract_violation_as_failed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let facade = StepFacade::new(|_| NextAgent::ContinueWave(vec![agent(0, false)]), 1);
    let llm = CountingLlm::default();

    let message = expect_err(run_audited_bounded(&facade, &llm, dir.path()).await);

    assert!(message.contains("contract violation"), "{message}");
    assert_bundle_failed(dir.path(), &message);
}

#[tokio::test]
async fn audited_run_records_a_progress_violation_as_failed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let facade = StepFacade::new(|_| NextAgent::Continue(agent(0, false)), 2)
        .with_mutation(Mutation::ClearAtLeast(1));
    let llm = CountingLlm::default();

    let message = expect_err(run_audited_bounded(&facade, &llm, dir.path()).await);

    assert!(message.contains("progress violation"), "{message}");
    assert_bundle_failed(dir.path(), &message);
}

#[tokio::test]
async fn audited_run_records_the_skip_limit_as_failed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let facade = StepFacade::new(|_| NextAgent::Skip("nothing to do".to_string()), 1);
    let llm = CountingLlm::default();

    let message = expect_err(run_audited_bounded(&facade, &llm, dir.path()).await);

    assert!(message.contains("consecutive NextAgent::Skip"), "{message}");
    assert_bundle_failed(dir.path(), &message);
}
