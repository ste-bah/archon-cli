
// ---------------------------------------------------------------------------
// Stub Facade — standard 3-agent pipeline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn progress_facades_keep_concurrent_run_channels_isolated() {
    let facade: Arc<dyn PipelineFacade> = Arc::new(StubFacade::new());
    let (first_tx, mut first_rx) = tokio::sync::mpsc::channel(1);
    let (second_tx, mut second_rx) = tokio::sync::mpsc::channel(1);
    let first = PipelineProgressFacade::new(Arc::clone(&facade), first_tx);
    let second = PipelineProgressFacade::new(Arc::clone(&facade), second_tx);
    let mut first_session = facade.init_session("first").await.unwrap();
    let mut second_session = facade.init_session("second").await.unwrap();
    let agent = StubFacade::make_agent(0);
    let result = AgentResult {
        output: "done".into(),
        tool_use_log: Vec::new(),
        tokens_in: Some(0),
        tokens_out: Some(0),
        cost_usd: 0.0,
        duration: Duration::ZERO,
        quality: None,
    };
    let quality = QualityScore {
        overall: 0.85,
        dimensions: HashMap::new(),
    };

    first
        .process_completion(&mut first_session, &agent, &result, &quality)
        .await
        .unwrap();
    second
        .process_completion(&mut second_session, &agent, &result, &quality)
        .await
        .unwrap();
    drop(first);

    assert_eq!(
        first_rx.recv().await.as_deref(),
        Some("[pipeline phase 1] Agent 1 complete (quality: 0.85)\n")
    );
    assert_eq!(first_rx.recv().await, None);
    assert_eq!(
        second_rx.recv().await.as_deref(),
        Some("[pipeline phase 1] Agent 1 complete (quality: 0.85)\n")
    );
    assert!(!second_rx.is_closed());
}

/// A minimal PipelineFacade implementation that cycles through 3 agents.
struct StubFacade {
    /// Tracks which agent index we are on (0, 1, 2, then Done).
    agent_index: Mutex<usize>,
}

impl StubFacade {
    fn new() -> Self {
        Self {
            agent_index: Mutex::new(0),
        }
    }

    fn make_agent(index: usize) -> AgentInfo {
        AgentInfo {
            key: format!("agent-{}", index + 1),
            display_name: format!("Agent {}", index + 1),
            model: "mock-model".to_string(),
            phase: index as u32 + 1,
            critical: true,
            parallelizable: false,
            quality_threshold: 0.7,
            tool_access_level: ToolAccessLevel::ReadOnly,
        }
    }
}

#[async_trait::async_trait]
impl PipelineFacade for StubFacade {
    async fn init_session(&self, task: &str) -> anyhow::Result<PipelineSession> {
        Ok(PipelineSession {
            id: "test-session-001".to_string(),
            pipeline_type: PipelineType::Coding,
            task: task.to_string(),
            started_at: Instant::now(),
            agent_results: vec![],
            leann_context: String::new(),
        })
    }

    async fn next_agent(&self, _session: &PipelineSession) -> anyhow::Result<NextAgent> {
        let mut idx = self.agent_index.lock().unwrap();
        if *idx < 3 {
            let agent = Self::make_agent(*idx);
            *idx += 1;
            Ok(NextAgent::Continue(agent))
        } else {
            Ok(NextAgent::Done)
        }
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
        // Return a single user message, empty system, empty tools.
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": format!("Do work for {}", agent.key),
        })];
        Ok((messages, vec![], vec![]))
    }

    async fn score_quality(
        &self,
        _session: &PipelineSession,
        _agent: &AgentInfo,
        _result: &AgentResult,
    ) -> anyhow::Result<QualityScore> {
        Ok(QualityScore {
            overall: 0.85,
            dimensions: HashMap::new(),
        })
    }

    async fn process_completion(
        &self,
        _session: &mut PipelineSession,
        _agent: &AgentInfo,
        _result: &AgentResult,
        _quality: &QualityScore,
    ) -> anyhow::Result<()> {
        // No-op for stub.
        Ok(())
    }

    async fn finalize(&self, session: PipelineSession) -> anyhow::Result<PipelineResult> {
        let duration = session.started_at.elapsed();
        let total_cost: f64 = session.agent_results.iter().map(|(_, r)| r.cost_usd).sum();

        Ok(PipelineResult {
            session_id: session.id.clone(),
            pipeline_type: session.pipeline_type.clone(),
            agent_results: session.agent_results,
            total_cost_usd: total_cost,
            duration,
            final_output: "Pipeline complete".to_string(),
        })
    }
}

struct WaveFacade;

#[async_trait::async_trait]
impl PipelineFacade for WaveFacade {
    async fn init_session(&self, task: &str) -> anyhow::Result<PipelineSession> {
        Ok(PipelineSession {
            id: "test-session-wave".to_string(),
            pipeline_type: PipelineType::Coding,
            task: task.to_string(),
            started_at: Instant::now(),
            agent_results: vec![],
            leann_context: String::new(),
        })
    }

    async fn next_agent(&self, session: &PipelineSession) -> anyhow::Result<NextAgent> {
        if session.agent_results.is_empty() {
            let mut wave = vec![StubFacade::make_agent(0), StubFacade::make_agent(1)];
            for agent in &mut wave {
                agent.parallelizable = true;
            }
            Ok(NextAgent::ContinueWave(wave))
        } else {
            Ok(NextAgent::Done)
        }
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
            vec![],
            vec![],
        ))
    }

    async fn score_quality(
        &self,
        _session: &PipelineSession,
        _agent: &AgentInfo,
        _result: &AgentResult,
    ) -> anyhow::Result<QualityScore> {
        Ok(QualityScore {
            overall: 0.85,
            dimensions: HashMap::new(),
        })
    }

    async fn process_completion(
        &self,
        _session: &mut PipelineSession,
        _agent: &AgentInfo,
        _result: &AgentResult,
        _quality: &QualityScore,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn finalize(&self, session: PipelineSession) -> anyhow::Result<PipelineResult> {
        Ok(PipelineResult {
            session_id: session.id.clone(),
            pipeline_type: session.pipeline_type.clone(),
            total_cost_usd: 0.0,
            duration: session.started_at.elapsed(),
            final_output: "wave complete".to_string(),
            agent_results: session.agent_results,
        })
    }
}

struct WaveRetryFacade;

#[async_trait::async_trait]
impl PipelineFacade for WaveRetryFacade {
    async fn init_session(&self, task: &str) -> anyhow::Result<PipelineSession> {
        Ok(PipelineSession {
            id: "test-session-wave-retry".to_string(),
            pipeline_type: PipelineType::Coding,
            task: task.to_string(),
            started_at: Instant::now(),
            agent_results: vec![],
            leann_context: String::new(),
        })
    }

    async fn next_agent(&self, session: &PipelineSession) -> anyhow::Result<NextAgent> {
        if session.agent_results.is_empty() {
            let mut wave = vec![StubFacade::make_agent(0), StubFacade::make_agent(1)];
            for agent in &mut wave {
                agent.parallelizable = true;
            }
            Ok(NextAgent::ContinueWave(wave))
        } else {
            Ok(NextAgent::Done)
        }
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
            vec![],
            vec![],
        ))
    }

    async fn score_quality(
        &self,
        _session: &PipelineSession,
        _agent: &AgentInfo,
        result: &AgentResult,
    ) -> anyhow::Result<QualityScore> {
        let overall = if result.output == "retry-ok" {
            0.85
        } else {
            0.10
        };
        Ok(QualityScore {
            overall,
            dimensions: HashMap::new(),
        })
    }

    async fn process_completion(
        &self,
        _session: &mut PipelineSession,
        _agent: &AgentInfo,
        _result: &AgentResult,
        _quality: &QualityScore,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn finalize(&self, session: PipelineSession) -> anyhow::Result<PipelineResult> {
        Ok(PipelineResult {
            session_id: session.id.clone(),
            pipeline_type: session.pipeline_type.clone(),
            total_cost_usd: 0.0,
            duration: session.started_at.elapsed(),
            final_output: "wave retry complete".to_string(),
            agent_results: session.agent_results,
        })
    }
}

// ---------------------------------------------------------------------------
// Stub Facade with Skip — skips agent-2
// ---------------------------------------------------------------------------

struct SkipFacade {
    agent_index: Mutex<usize>,
}

impl SkipFacade {
    fn new() -> Self {
        Self {
            agent_index: Mutex::new(0),
        }
    }
}

#[async_trait::async_trait]
impl PipelineFacade for SkipFacade {
    async fn init_session(&self, task: &str) -> anyhow::Result<PipelineSession> {
        Ok(PipelineSession {
            id: "test-session-skip".to_string(),
            pipeline_type: PipelineType::Coding,
            task: task.to_string(),
            started_at: Instant::now(),
            agent_results: vec![],
            leann_context: String::new(),
        })
    }

    async fn next_agent(&self, _session: &PipelineSession) -> anyhow::Result<NextAgent> {
        let mut idx = self.agent_index.lock().unwrap();
        if *idx >= 3 {
            return Ok(NextAgent::Done);
        }
        let current = *idx;
        *idx += 1;
        if current == 1 {
            // Skip agent-2
            Ok(NextAgent::Skip("agent-2 not needed".to_string()))
        } else {
            Ok(NextAgent::Continue(StubFacade::make_agent(current)))
        }
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
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": format!("Do work for {}", agent.key),
        })];
        Ok((messages, vec![], vec![]))
    }

    async fn score_quality(
        &self,
        _session: &PipelineSession,
        _agent: &AgentInfo,
        _result: &AgentResult,
    ) -> anyhow::Result<QualityScore> {
        Ok(QualityScore {
            overall: 0.85,
            dimensions: HashMap::new(),
        })
    }

    async fn process_completion(
        &self,
        _session: &mut PipelineSession,
        _agent: &AgentInfo,
        _result: &AgentResult,
        _quality: &QualityScore,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn finalize(&self, session: PipelineSession) -> anyhow::Result<PipelineResult> {
        let duration = session.started_at.elapsed();
        let total_cost: f64 = session.agent_results.iter().map(|(_, r)| r.cost_usd).sum();
        Ok(PipelineResult {
            session_id: session.id.clone(),
            pipeline_type: session.pipeline_type.clone(),
            agent_results: session.agent_results,
            total_cost_usd: total_cost,
            duration,
            final_output: "Pipeline complete (with skip)".to_string(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Verify that a 3-agent stub pipeline runs end-to-end without errors and
/// the runner calls the LLM exactly 3 times.
#[tokio::test]
async fn test_three_agent_pipeline_executes_end_to_end() {
    let facade = StubFacade::new();
    let llm = MockLlmClient::new("Agent output OK");

    let result = run_pipeline(&facade, &llm, "Implement feature X", None, None, None)
        .await
        .expect("pipeline should complete without error");

    // The mock was called exactly 3 times (once per agent).
    assert_eq!(
        llm.recorded_message_counts().len(),
        3,
        "LLM should have been called exactly 3 times"
    );

    // The result should contain a valid session id.
    assert_eq!(result.session_id, "test-session-001");
    assert_eq!(result.final_output, "Pipeline complete");
}
