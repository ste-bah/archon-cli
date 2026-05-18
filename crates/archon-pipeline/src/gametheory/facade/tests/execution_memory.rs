use super::super::*;
use super::*;

#[test]
fn test_specialist_fixture_outputs_non_empty() {
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms set quantities simultaneously.",
        None,
    ))
    .unwrap();

    // Build a minimal routing decision to test fixture execution.
    let rd = RoutingDecision {
        run_id: "test-fixture-run".into(),
        fingerprint_id: fp.run_id.clone(),
        enabled_specialists: vec![
            "nash-equilibrium-finder".into(),
            "payoff-matrix-builder".into(),
        ],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-01-01T00:00:00Z".into(),
    };

    let (outputs, failed, audits) = execute_test_specialist_fixture(
        &rd,
        &fp,
        "Two firms set quantities.",
        &GameTheoryMemoryContext::default(),
    );
    assert_eq!(outputs.len(), 2);
    assert!(
        outputs
            .get("nash-equilibrium-finder")
            .unwrap()
            .contains("nash-equilibrium-finder")
    );
    assert!(
        outputs
            .get("payoff-matrix-builder")
            .unwrap()
            .contains("payoff-matrix-builder")
    );
    assert!(
        failed.is_empty(),
        "no forced failures without the test hook suffix"
    );
    assert_eq!(audits.len(), 2);
}
#[test]
fn test_failure_isolation_with_force_fail_suffix() {
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms set quantities simultaneously.",
        None,
    ))
    .unwrap();

    let rd = RoutingDecision {
        run_id: "test-fail-iso".into(),
        fingerprint_id: fp.run_id.clone(),
        enabled_specialists: vec![
            "nash-equilibrium-finder".into(),
            "bayesian-game-analyzer-FORCE-FAIL-FOR-TEST".into(),
            "payoff-matrix-builder".into(),
        ],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-01-01T00:00:00Z".into(),
    };

    let (outputs, failed, audits) = execute_test_specialist_fixture(
        &rd,
        &fp,
        "Two firms set quantities.",
        &GameTheoryMemoryContext::default(),
    );
    // 2 of 3 succeed
    assert_eq!(outputs.len(), 2);
    assert!(outputs.contains_key("nash-equilibrium-finder"));
    assert!(outputs.contains_key("payoff-matrix-builder"));
    // 1 fails due to test hook
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, "bayesian-game-analyzer-FORCE-FAIL-FOR-TEST");
    assert!(failed[0].1.contains("forced failure"));
    assert_eq!(audits.len(), 3);
}
#[test]
fn test_facade_recalls_memory_for_specialist_keys() {
    let graph = archon_memory::MemoryGraph::in_memory().unwrap();
    graph
        .store_memory(
            "stored payoff prior context",
            "gametheory/tier1/payoffs",
            archon_memory::MemoryType::Fact,
            0.9,
            &[TAG_GAMETHEORY_PIPELINE.to_string()],
            "test",
            "workspace",
        )
        .unwrap();
    let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> = std::sync::Arc::new(graph);
    let ctx = GameTheoryMemoryContext::new(memory, None, true);
    let llm = canned_pipeline_llm();
    let rd = RoutingDecision {
        run_id: "run-memory".into(),
        fingerprint_id: "fp-memory".into(),
        enabled_specialists: vec!["nash-equilibrium-finder".into()],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };

    let (_outputs, failed, audits) = block_on(execute_specialists_real(
        &llm,
        &rd,
        &test_fingerprint("fp-memory"),
        "Two firms choose prices.",
        &ctx,
    ))
    .unwrap();

    assert!(failed.is_empty());
    assert!(llm.prompts()[0].contains("stored payoff prior context"));
    assert_eq!(audits[0].agent_key, "nash-equilibrium-finder");
    assert_eq!(audits[0].cozo_hits, 1);
    assert_eq!(audits[0].leann_hits, 0);
}

#[test]
fn test_real_specialist_loads_prompt_file_and_uses_registry_model() {
    let llm = canned_specialist_llm();
    let rd = RoutingDecision {
        run_id: "run-prompt-model".into(),
        fingerprint_id: "fp-prompt-model".into(),
        enabled_specialists: vec!["nash-equilibrium-finder".into()],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };

    let (_outputs, failed, _audits) = block_on(execute_specialists_real(
        &llm,
        &rd,
        &test_fingerprint("fp-prompt-model"),
        "Two firms choose prices.",
        &GameTheoryMemoryContext::default(),
    ))
    .unwrap();

    assert!(failed.is_empty());
    assert!(llm.prompts()[0].contains("Nash-Seeker"));
    assert_eq!(llm.models()[0], "opus");
}

#[test]
fn test_specialist_fixture_recalls_memory_for_debug_path() {
    let graph = archon_memory::MemoryGraph::in_memory().unwrap();
    graph
        .store_memory(
            "stored no-provider prior context",
            "gametheory/tier1/payoffs",
            archon_memory::MemoryType::Fact,
            0.9,
            &[TAG_GAMETHEORY_PIPELINE.to_string()],
            "test",
            "workspace",
        )
        .unwrap();
    let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> = std::sync::Arc::new(graph);
    let ctx = GameTheoryMemoryContext::new(memory, None, true);
    let rd = RoutingDecision {
        run_id: "run-fixture-memory".into(),
        fingerprint_id: "fp-fixture-memory".into(),
        enabled_specialists: vec!["nash-equilibrium-finder".into()],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };

    let (outputs, failed, audits) = execute_test_specialist_fixture(
        &rd,
        &test_fingerprint("fp-fixture-memory"),
        "Two firms choose prices.",
        &ctx,
    );

    assert!(failed.is_empty());
    assert!(outputs["nash-equilibrium-finder"].contains("stored no-provider prior context"));
    assert_eq!(audits[0].cozo_hits, 1);
    assert_eq!(audits[0].leann_hits, 0);
}
#[test]
fn test_facade_falls_back_to_leann_when_cozo_empty() {
    let graph = archon_memory::MemoryGraph::in_memory().unwrap();
    let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> = std::sync::Arc::new(graph);
    let leann = std::sync::Arc::new(MockLeannSearcher::new("leann semantic prior"));
    let ctx = GameTheoryMemoryContext::new(memory, Some(leann.clone()), true);
    let llm = canned_pipeline_llm();
    let rd = RoutingDecision {
        run_id: "run-leann".into(),
        fingerprint_id: "fp-leann".into(),
        enabled_specialists: vec!["nash-equilibrium-finder".into()],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };

    let (_outputs, _failed, audits) = block_on(execute_specialists_real(
        &llm,
        &rd,
        &test_fingerprint("fp-leann"),
        "Two firms choose prices.",
        &ctx,
    ))
    .unwrap();

    assert!(llm.prompts()[0].contains("leann semantic prior"));
    assert_eq!(audits[0].cozo_hits, 0);
    assert_eq!(audits[0].leann_hits, 2);
    assert_eq!(leann.calls(), 2);
}
#[test]
fn test_no_recall_when_memory_keys_empty() {
    let graph = archon_memory::MemoryGraph::in_memory().unwrap();
    let memory: std::sync::Arc<dyn archon_memory::MemoryTrait> = std::sync::Arc::new(graph);
    let leann = std::sync::Arc::new(MockLeannSearcher::new("should not be used"));
    let ctx = GameTheoryMemoryContext::new(memory, Some(leann.clone()), true);
    let llm = canned_specialist_llm();
    let rd = RoutingDecision {
        run_id: "run-empty".into(),
        fingerprint_id: "fp-empty".into(),
        enabled_specialists: vec!["agent-with-no-memory-keys".into()],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };

    let (_outputs, _failed, audits) = block_on(execute_specialists_real(
        &llm,
        &rd,
        &test_fingerprint("fp-empty"),
        "Two firms choose prices.",
        &ctx,
    ))
    .unwrap();

    assert_eq!(audits[0].memory_keys.len(), 0);
    assert_eq!(audits[0].cozo_hits, 0);
    assert_eq!(audits[0].leann_hits, 0);
    assert_eq!(leann.calls(), 0);
    assert!(!llm.prompts()[0].contains("## Prior Context"));
}
#[test]
fn test_cost_tracked_per_specialist() {
    let llm = canned_specialist_llm();
    let rd = RoutingDecision {
        run_id: "run-cost".into(),
        fingerprint_id: "fp-cost".into(),
        enabled_specialists: vec!["nash-equilibrium-finder".into()],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };

    let outcome = block_on(execute_specialists_real_with_options(
        &llm,
        &rd,
        &test_fingerprint("fp-cost"),
        "Two firms choose prices.",
        &GameTheoryMemoryContext::default(),
        &GameTheoryRunOptions::default(),
    ))
    .unwrap();

    let cost = outcome.costs_usd["nash-equilibrium-finder"];
    assert!(cost > 0.0);
    assert!((outcome.total_cost_usd - cost).abs() < f64::EPSILON);
    assert!((outcome.tier_costs_usd[&2] - cost).abs() < f64::EPSILON);
}
#[test]
fn test_concurrency_cap_respected_per_run_flag() {
    let llm = canned_specialist_llm();
    let rd = RoutingDecision {
        run_id: "run-concurrency".into(),
        fingerprint_id: "fp-concurrency".into(),
        enabled_specialists: vec![
            "nash-equilibrium-finder".into(),
            "payoff-matrix-builder".into(),
        ],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };

    let outcome = block_on(execute_specialists_real_with_options(
        &llm,
        &rd,
        &test_fingerprint("fp-concurrency"),
        "Two firms choose prices.",
        &GameTheoryMemoryContext::default(),
        &GameTheoryRunOptions {
            budget_usd: 20.0,
            max_concurrent: 1,
            style_profile_id: None,
            enable_tier11: false,
            kb_pack_id: None,
        },
    ))
    .unwrap();

    assert!(outcome.max_observed_concurrent <= 1);
    assert_eq!(outcome.outputs.len(), 2);
}
#[test]
fn test_specialists_dispatch_in_parallel_waves() {
    let llm = SlowTier1LlmClient::new("parallel specialist output".to_string());
    let rd = RoutingDecision {
        run_id: "run-specialist-parallel".into(),
        fingerprint_id: "fp-specialist-parallel".into(),
        enabled_specialists: vec![
            "nash-equilibrium-finder".into(),
            "payoff-matrix-builder".into(),
            "dominant-strategy-identifier".into(),
        ],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-05-03T00:00:00Z".into(),
    };

    let started = std::time::Instant::now();
    let outcome = block_on(execute_specialists_real_with_options(
        &llm,
        &rd,
        &test_fingerprint("fp-specialist-parallel"),
        "Two firms choose prices.",
        &GameTheoryMemoryContext::default(),
        &GameTheoryRunOptions {
            budget_usd: 20.0,
            max_concurrent: 2,
            style_profile_id: None,
            enable_tier11: false,
            kb_pack_id: None,
        },
    ))
    .unwrap();

    assert_eq!(outcome.outputs.len(), 3);
    assert_eq!(outcome.max_observed_concurrent, 2);
    assert!(
        llm.max_active() > 1,
        "specialist calls must overlap when max_concurrent > 1"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_millis(320),
        "three 120ms specialists with max_concurrent=2 should not run serially"
    );
}
#[test]
fn test_tier1_foundation_agents_run_as_parallel_wave() {
    let llm = SlowTier1LlmClient::new(canned_fingerprint_json());
    let started = std::time::Instant::now();
    let (fp, audits) = block_on(execute_tier1_real(
        &llm,
        "run-tier1-parallel",
        "Two firms simultaneously set prices in a Bertrand duopoly.",
        "2026-05-04T00:00:00Z",
        &GameTheoryMemoryContext::default(),
    ))
    .unwrap();

    assert_eq!(fp.primary_family, "Bertrand competition");
    assert_eq!(audits.len(), TIER1_MEMORY_AGENT_KEYS.len());
    assert!(
        llm.max_active() > 1,
        "Tier 1 mandatory agents must overlap in one parallel wave"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_millis(360),
        "four 120ms Tier 1 calls should not run serially"
    );
    let prompts = llm.prompts().join("\n");
    for agent_key in TIER1_MEMORY_AGENT_KEYS {
        assert!(prompts.contains(agent_key));
    }
}
#[test]
fn test_real_specialist_execution_with_failure_isolation() {
    let db = test_db();
    let fp = block_on(classify(
        &db,
        "Two firms set quantities simultaneously.",
        None,
    ))
    .unwrap();

    let rd = RoutingDecision {
        run_id: "test-real-fail-iso".into(),
        fingerprint_id: fp.run_id.clone(),
        enabled_specialists: vec![
            "market-structure-analyzer".into(),
            "game-tree-builder-FORCE-FAIL-FOR-TEST".into(),
            "payoff-matrix-builder".into(),
        ],
        skipped_specialists: vec![],
        evaluated_conditions: vec![],
        created_at: "2026-01-01T00:00:00Z".into(),
    };

    let (outputs, failed, audits) = execute_test_specialist_fixture(
        &rd,
        &fp,
        "Two firms set quantities.",
        &GameTheoryMemoryContext::default(),
    );
    assert_eq!(outputs.len(), 2, "2 of 3 specialists should succeed");
    assert!(outputs.contains_key("market-structure-analyzer"));
    assert!(outputs.contains_key("payoff-matrix-builder"));
    assert_eq!(
        failed.len(),
        1,
        "1 specialist should fail due to FORCE-FAIL hook"
    );
    assert_eq!(failed[0].0, "game-tree-builder-FORCE-FAIL-FOR-TEST");
    assert!(
        failed[0].1.contains("forced failure"),
        "error message should mention forced failure"
    );

    // Verify the failed specialist is NOT in outputs
    assert!(!outputs.contains_key("game-tree-builder-FORCE-FAIL-FOR-TEST"));
    assert_eq!(audits.len(), 3);
}
