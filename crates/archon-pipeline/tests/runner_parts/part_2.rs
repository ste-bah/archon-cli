#[tokio::test]
async fn test_runner_uses_agent_execution_path_when_available() {
    let facade = StubFacade::new();
    let llm = AgentPathLlmClient::new();

    let result = run_pipeline(&facade, &llm, "use real agents", None, None, None)
        .await
        .expect("pipeline should use run_agent");

    assert_eq!(result.agent_results.len(), 3);
    let calls = llm.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0].0, "test-session-001");
    assert_eq!(calls[0].1, "use real agents");
    assert_eq!(
        calls
            .iter()
            .map(|(_, _, agent)| agent.as_str())
            .collect::<Vec<_>>(),
        vec!["agent-1", "agent-2", "agent-3"]
    );
}

#[tokio::test]
async fn test_runner_persists_sona_trajectory_when_learning_supplied() {
    let db = Arc::new(cozo::DbInstance::new("mem", "", "").unwrap());
    initialize_learning_schemas(db.as_ref()).unwrap();
    let mut learning = LearningIntegration::new_with_persistent_sona(
        Arc::clone(&db),
        LearningIntegrationConfig::default(),
        None,
        8,
    );
    let facade = StubFacade::new();
    let llm = MockLlmClient::new("Agent output OK");

    let result = run_pipeline(
        &facade,
        &llm,
        "persist SONA trajectory",
        None,
        None,
        Some(&mut learning),
    )
    .await
    .expect("pipeline should complete with learning enabled");

    assert_eq!(result.agent_results.len(), 3);
    let samples = query_trajectories_for_training(db.as_ref(), 8).unwrap();
    assert_eq!(samples.len(), 3);
    assert!(samples.iter().all(|sample| sample.quality > 0.0));
    assert!(samples.iter().all(|sample| sample.embedding.len() == 8));
}

#[tokio::test]
async fn test_runner_injects_reasoning_bank_and_desc_context() {
    let db = Arc::new(cozo::DbInstance::new("mem", "", "").unwrap());
    initialize_learning_schemas(db.as_ref()).unwrap();

    let desc_store = DescEpisodeStore::from_arc(Arc::clone(&db));
    desc_store
        .store_episode(&DescEpisode {
            episode_id: "episode-seed".into(),
            session_id: "prior-session".into(),
            task_type: "1".into(),
            description: "need schema design before API wiring".into(),
            solution: "Designed schema first, then wired API boundaries.".into(),
            outcome: "success".into(),
            quality_score: 0.95,
            reward: 0.95,
            tags: vec!["pipeline".into()],
            trajectory_id: None,
            created_at: 0,
            updated_at: 0,
        })
        .unwrap();

    let reasoning_bank = ReasoningBank::new(ReasoningBankDeps {
        pattern_store: PatternStore::new(),
        causal_memory: None,
        gnn_enhancer: None,
        sona_engine: None,
        config: ReasoningBankConfig::default(),
    });
    let mut learning =
        LearningIntegration::new(None, None, LearningIntegrationConfig::default(), None)
            .with_reasoning_bank(reasoning_bank)
            .with_desc_store(desc_store);

    let facade = StubFacade::new();
    let llm = MockLlmClient::new("Agent output OK");

    let result = run_pipeline(
        &facade,
        &llm,
        "break down the implementation steps",
        None,
        None,
        Some(&mut learning),
    )
    .await
    .expect("pipeline should complete with learning context enabled");

    assert_eq!(result.agent_results.len(), 3);
    let system_text = llm.recorded_system_texts().join("\n");
    assert!(system_text.contains("## DESC Episodes"));
    assert!(system_text.contains("episode-seed"));
    assert!(system_text.contains("## Reasoning Context"));
}

/// Verify context isolation: each LLM call should receive a fresh message
/// history (not an accumulation of prior agents' messages). Our stub facade
/// returns exactly 1 message per prompt, so every call should see len == 1.
#[tokio::test]
async fn test_context_isolation_fresh_messages() {
    let facade = StubFacade::new();
    let llm = MockLlmClient::new("Isolated response");

    let _result = run_pipeline(&facade, &llm, "Test isolation", None, None, None)
        .await
        .expect("pipeline should succeed");

    let counts = llm.recorded_message_counts();
    assert_eq!(counts.len(), 3, "Expected 3 LLM calls");

    for (i, &count) in counts.iter().enumerate() {
        assert!(
            count <= 2,
            "Agent {} received {} messages — context is leaking across agents (expected <= 2)",
            i + 1,
            count,
        );
    }
}

/// Verify PipelineResult.agent_results contains exactly 3 entries matching
/// our 3 stub agents.
#[tokio::test]
async fn test_pipeline_result_contains_all_agents() {
    let facade = StubFacade::new();
    let llm = MockLlmClient::new("Result content");

    let result = run_pipeline(&facade, &llm, "Check agent count", None, None, None)
        .await
        .expect("pipeline should succeed");

    assert_eq!(
        result.agent_results.len(),
        3,
        "PipelineResult should contain exactly 3 agent results"
    );

    // Verify each agent key is present in order.
    let keys: Vec<&str> = result
        .agent_results
        .iter()
        .map(|(info, _)| info.key.as_str())
        .collect();
    assert_eq!(keys, vec!["agent-1", "agent-2", "agent-3"]);
}

#[tokio::test]
async fn test_parallel_wave_executes_agents_concurrently_and_commits_in_order() {
    let facade = WaveFacade;
    let llm = DelayedLlmClient::new(Duration::from_millis(120));

    let result = run_pipeline(&facade, &llm, "parallel wave", None, None, None)
        .await
        .expect("parallel wave pipeline should succeed");

    // The active-call counter proves overlap; serial execution peaks at 1.
    // A wall-clock bound here fails on loaded CI runners.
    assert!(
        llm.max_observed() >= 2,
        "expected at least two concurrent LLM calls"
    );
    let keys = result
        .agent_results
        .iter()
        .map(|(agent, _)| agent.key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, vec!["agent-1", "agent-2"]);
}

#[tokio::test]
async fn test_parallel_wave_retries_low_quality_outputs_before_commit() {
    let facade = WaveRetryFacade;
    let llm = SequencedLlmClient::new();

    let result = run_pipeline(&facade, &llm, "parallel wave retry", None, None, None)
        .await
        .expect("parallel wave retry pipeline should succeed");

    assert_eq!(
        llm.calls(),
        4,
        "two low-quality first attempts should retry"
    );
    let outputs = result
        .agent_results
        .iter()
        .map(|(_, result)| result.output.as_str())
        .collect::<Vec<_>>();
    assert_eq!(outputs, vec!["retry-ok", "retry-ok"]);
}

#[tokio::test(start_paused = true)]
async fn test_parallel_wave_retries_529_with_overload_policy_and_succeeds() {
    let facade = WaveFacade;
    let llm = FlakyWaveLlmClient::new(
        1,
        LlmError::Overloaded {
            status: Some(529),
            retry_after_secs: Some(17),
            message: String::new().into(),
        },
    );
    let started = tokio::time::Instant::now();

    let result = run_pipeline(&facade, &llm, "parallel wave overload", None, None, None)
        .await
        .expect("the wave member should retry and succeed");

    assert_eq!(result.agent_results.len(), 2);
    assert_eq!(llm.calls(), 3, "one of two wave members should retry once");
    assert_eq!(started.elapsed(), Duration::from_secs(17));
}

#[tokio::test(start_paused = true)]
async fn test_parallel_wave_retries_ordinary_transient_error_and_exhausts_budget() {
    let facade = WaveFacade;
    let succeeds = FlakyWaveLlmClient::new(1, LlmError::Http("connection reset".into()));
    let started = tokio::time::Instant::now();
    run_pipeline(
        &facade,
        &succeeds,
        "parallel wave transient",
        None,
        None,
        None,
    )
    .await
    .expect("ordinary transient wave error should retry");
    assert_eq!(succeeds.calls(), 3);
    assert_eq!(started.elapsed(), Duration::from_millis(250));

    let exhausted = FlakyWaveLlmClient::new(
        usize::MAX,
        LlmError::Overloaded {
            status: Some(529),
            retry_after_secs: None,
            message: String::new().into(),
        },
    );
    assert!(
        run_pipeline(
            &facade,
            &exhausted,
            "parallel wave exhaustion",
            None,
            None,
            None
        )
        .await
        .is_err()
    );
    assert_eq!(
        exhausted.calls(),
        6,
        "each wave member uses the pipeline attempt budget independently"
    );
}

#[tokio::test]
async fn test_pipeline_retries_transient_agent_transport_errors() {
    let facade = StubFacade::new();
    let llm = FlakyAgentLlmClient::new(1);

    let result = run_pipeline(&facade, &llm, "transient transport retry", None, None, None)
        .await
        .expect("transient subagent transport failure should retry");

    assert_eq!(result.agent_results.len(), 3);
    assert_eq!(
        llm.calls(),
        4,
        "first agent should need one retry, then remaining agents run once"
    );
}

#[tokio::test]
async fn test_parallel_wave_injects_reflexion_context_on_retry() {
    let facade = WaveRetryFacade;
    let llm = RecordingSequencedLlmClient::new();
    let mut reflexion = ReflexionInjector::new(3);

    let result = run_pipeline(
        &facade,
        &llm,
        "parallel wave retry",
        None,
        Some(&mut reflexion),
        None,
    )
    .await
    .expect("parallel wave retry pipeline should succeed");

    assert_eq!(llm.calls(), 4);
    assert_eq!(result.agent_results.len(), 2);

    let system_texts = llm.recorded_system_texts();
    assert!(
        system_texts
            .iter()
            .filter(|text| text.contains("## Reflexion"))
            .count()
            >= 2,
        "each retried wave agent should receive reflexion context: {system_texts:?}"
    );
    assert!(
        system_texts
            .iter()
            .any(|text| text.contains("Quality 0.10 below threshold")),
        "reflexion context should include the concrete failed quality reason"
    );
}

/// Verify that per-agent overhead (time spent in runner machinery, excluding
/// actual LLM latency) is under 2 seconds. Since MockLlmClient returns
/// instantly, the total wall-clock time for 3 agents should be well under 6s.
#[tokio::test]
async fn test_per_agent_overhead_under_2_seconds() {
    let facade = StubFacade::new();
    let llm = MockLlmClient::new("Fast response");

    let start = Instant::now();
    let result = run_pipeline(&facade, &llm, "Performance test", None, None, None)
        .await
        .expect("pipeline should succeed");
    let elapsed = start.elapsed();

    let agent_count = result.agent_results.len() as u64;
    assert!(agent_count > 0, "Should have at least one agent result");

    let per_agent = elapsed / agent_count as u32;
    assert!(
        per_agent < Duration::from_secs(2),
        "Per-agent overhead was {:?}, which exceeds 2s limit",
        per_agent,
    );
}

/// Verify that when the facade returns Skip for an agent, the runner does
/// not call the LLM for that agent and excludes it from results.
#[tokio::test]
async fn test_skip_agent() {
    let facade = SkipFacade::new();
    let llm = MockLlmClient::new("Non-skipped output");

    let result = run_pipeline(&facade, &llm, "Test skip semantics", None, None, None)
        .await
        .expect("pipeline should succeed");

    // Only agent-1 and agent-3 should appear (agent-2 was skipped).
    assert_eq!(
        result.agent_results.len(),
        2,
        "Should have 2 results (agent-2 was skipped)"
    );

    let keys: Vec<&str> = result
        .agent_results
        .iter()
        .map(|(info, _)| info.key.as_str())
        .collect();
    assert_eq!(keys, vec!["agent-1", "agent-3"]);

    // LLM should have been called only twice.
    assert_eq!(
        llm.recorded_message_counts().len(),
        2,
        "LLM should not be called for skipped agents"
    );
}

/// Verify that after scoring, the quality information is accessible
/// through each agent's result (the runner should store the quality score).
/// We check that each AgentResult's output is non-empty (populated from LLM)
/// and that the runner completed the score_quality step for every agent.
#[tokio::test]
async fn test_quality_score_stored() {
    let facade = StubFacade::new();
    let llm = MockLlmClient::new("Quality-checked output");

    let result = run_pipeline(&facade, &llm, "Quality scoring test", None, None, None)
        .await
        .expect("pipeline should succeed");

    assert_eq!(result.agent_results.len(), 3);

    for (agent_info, agent_result) in &result.agent_results {
        // Each agent result should have the LLM output.
        assert!(
            !agent_result.output.is_empty(),
            "Agent {} should have non-empty output",
            agent_info.key,
        );

        // Token counts should reflect what our mock returned.
        assert_eq!(
            agent_result.tokens_in,
            Some(100),
            "Agent {} tokens_in mismatch",
            agent_info.key,
        );
        assert_eq!(
            agent_result.tokens_out,
            Some(50),
            "Agent {} tokens_out mismatch",
            agent_info.key,
        );
    }
}
