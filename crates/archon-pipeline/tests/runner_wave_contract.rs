//! The runner enforces the `NextAgent::ContinueWave` contract and never lets
//! a step complete without recording its results.
//!
//! Each run is wrapped in a short timeout, and the facade yields on every
//! `next_agent` call, so a regression to the old silent-drop loop fails fast
//! instead of hanging the test binary.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use archon_pipeline::runner::{
    AgentInfo, AgentResult, LlmClient, LlmResponse, NextAgent, PARALLEL_WAVE_LIMIT, PipelineFacade,
    PipelineResult, PipelineSession, PipelineType, QualityScore, ToolAccessLevel, run_pipeline,
};

const RUN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_NEXT_AGENT_CALLS: usize = 50;

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

/// Emits one fixed step until the session holds `target` results.
struct StepFacade {
    step: fn() -> NextAgent,
    target: usize,
    /// Simulates a facade hook that discards recorded results.
    clear_results_on_completion: bool,
    next_agent_calls: AtomicUsize,
}

impl StepFacade {
    fn new(step: fn() -> NextAgent, target: usize) -> Self {
        Self {
            step,
            target,
            clear_results_on_completion: false,
            next_agent_calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait::async_trait]
impl PipelineFacade for StepFacade {
    async fn init_session(&self, task: &str) -> anyhow::Result<PipelineSession> {
        Ok(PipelineSession {
            id: "wave-contract".to_string(),
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
        Ok((self.step)())
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
        if self.clear_results_on_completion {
            session.agent_results.clear();
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

fn expect_err(result: anyhow::Result<PipelineResult>) -> String {
    match result {
        Ok(_) => panic!("the run must fail"),
        Err(error) => error.to_string(),
    }
}

#[tokio::test]
async fn wave_of_non_parallelizable_agents_is_a_contract_error() {
    let facade = StepFacade::new(
        || NextAgent::ContinueWave(vec![agent(0, false), agent(1, false)]),
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
        || NextAgent::ContinueWave(vec![agent(0, true), agent(1, false)]),
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
        || NextAgent::ContinueWave((0..=PARALLEL_WAVE_LIMIT).map(|i| agent(i, true)).collect()),
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
    let facade = StepFacade::new(|| NextAgent::ContinueWave(Vec::new()), 1);
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(message.contains("contract violation"), "{message}");
    assert!(message.contains("the wave is empty"), "{message}");
}

#[tokio::test]
async fn wave_at_the_limit_runs_every_member() {
    let facade = StepFacade::new(
        || NextAgent::ContinueWave((0..PARALLEL_WAVE_LIMIT).map(|i| agent(i, true)).collect()),
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
    let mut facade = StepFacade::new(|| NextAgent::Continue(agent(0, false)), 2);
    facade.clear_results_on_completion = true;
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(message.contains("progress violation"), "{message}");
    assert!(
        message.contains("NextAgent::Continue [agent-0]"),
        "{message}"
    );
    assert!(message.contains("went from 1 to 1"), "{message}");
}

#[tokio::test]
async fn wave_that_loses_a_result_fails_instead_of_looping() {
    let mut facade = StepFacade::new(
        || NextAgent::ContinueWave(vec![agent(0, true), agent(1, true)]),
        2,
    );
    facade.clear_results_on_completion = true;
    let llm = CountingLlm::default();

    let message = expect_err(run_bounded(&facade, &llm).await);

    assert!(message.contains("progress violation"), "{message}");
    assert!(
        message.contains("NextAgent::ContinueWave [agent-0, agent-1]"),
        "{message}"
    );
    assert!(message.contains("expected 2 new result(s)"), "{message}");
}
