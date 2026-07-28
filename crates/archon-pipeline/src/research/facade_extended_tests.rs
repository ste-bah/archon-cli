use super::*;

// 10. facade with learning implements trait
#[test]
fn facade_with_learning_implements_trait() {
    use crate::learning::integration::PhDLearningIntegration;
    let learning = PhDLearningIntegration::new();
    let facade = ResearchFacade::with_learning(make_memory(), None, String::new(), None, learning);
    let _: &dyn PipelineFacade = &facade;
}

// 11. to_agent_info correctly converts
#[test]
fn to_agent_info_conversion() {
    let agent = &RESEARCH_AGENTS[0];
    let info = ResearchFacade::to_agent_info(
        agent,
        &archon_core::config::AnthropicModelsConfig::default(),
    );

    assert_eq!(info.key, "step-back-analyzer");
    assert_eq!(info.display_name, "Step-Back Analyzer");
    assert_eq!(info.phase, 1);
    assert_eq!(info.tool_access_level, ToolAccessLevel::ReadOnly);

    let writer = &RESEARCH_AGENTS[28];
    let writer_info = ResearchFacade::to_agent_info(
        writer,
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    assert_eq!(writer_info.tool_access_level, ToolAccessLevel::Full);
}

// 12. LEANN fallback triggers when memory key is missing
#[tokio::test]
async fn leann_fallback_on_missing_key() {
    let (facade, leann) = make_facade_with_leann("LEANN fallback result");

    let result = facade.recall_memory("research/nonexistent/key");
    assert_eq!(result, "LEANN fallback result");
    assert_eq!(leann.calls(), 1);
}

// 13. LEANN fallback NOT called when key exists
#[tokio::test]
async fn leann_not_called_when_key_exists() {
    let (facade, leann) = make_facade_with_leann("should not be used");

    // Store first via process_completion
    let mut session = facade.init_session("test").await.unwrap();
    let agent_info = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[0],
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let result = make_agent_result("stored content");
    let quality = QualityScore {
        overall: 0.75,
        dimensions: HashMap::new(),
    };
    facade
        .process_completion(&mut session, &agent_info, &result, &quality)
        .await
        .unwrap();

    let recalled = facade.recall_memory("research/foundation/framing");
    assert_eq!(recalled, "stored content");
    assert_eq!(leann.calls(), 0);
}

// 14. Memory persistence across two facades sharing the same MemoryTrait
#[tokio::test]
async fn memory_persists_across_facades_with_same_backend() {
    let memory = make_memory();
    let facade_a = ResearchFacade::new(Arc::clone(&memory), None, String::new(), None);
    let facade_b = ResearchFacade::new(Arc::clone(&memory), None, String::new(), None);

    // Store via facade A
    let mut session = facade_a.init_session("test").await.unwrap();
    let agent_info = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[0],
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let result = make_agent_result("persistent output");
    let quality = QualityScore {
        overall: 0.75,
        dimensions: HashMap::new(),
    };
    facade_a
        .process_completion(&mut session, &agent_info, &result, &quality)
        .await
        .unwrap();

    // Recall via facade B (same backend, different facade)
    let recalled = facade_b.recall_memory("research/foundation/framing");
    assert_eq!(recalled, "persistent output");
}

// 15. store_memory uses phd-pipeline tags
#[tokio::test]
async fn store_memory_uses_phd_pipeline_tags() {
    let facade = make_facade();
    let mut session = facade.init_session("test").await.unwrap();
    let agent_info = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[0],
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let result = make_agent_result("tagged output");
    let quality = QualityScore {
        overall: 0.75,
        dimensions: HashMap::new(),
    };
    facade
        .process_completion(&mut session, &agent_info, &result, &quality)
        .await
        .unwrap();

    // Verify tag search works
    let filter = SearchFilter {
        tags: vec![TAG_PHD_PIPELINE.to_string()],
        ..Default::default()
    };
    let results = facade.memory.search_memories(&filter).unwrap();
    assert!(
        !results.is_empty(),
        "should find memories with phd-pipeline tag"
    );
    let found = results
        .iter()
        .any(|m| m.title == "research/foundation/framing");
    assert!(found, "should find the stored memory by title");
}

#[tokio::test]
async fn consistency_prompt_gets_archon_runtime_context() {
    let facade = make_facade();
    let mut session = facade.init_session("test").await.unwrap();
    for (key, output) in [
        (
            "dissertation-architect",
            "**Total Chapters**: 8\n\n## Chapter 1: Introduction",
        ),
        (
            "introduction-writer",
            "Chapter 1 introduces the study and previews Chapter 2.",
        ),
        (
            "literature-review-writer",
            "Chapter 2 reviews literature and supports Chapter 3.",
        ),
    ] {
        let agent = get_agent_by_key(key).unwrap();
        let info = ResearchFacade::to_agent_info(
            agent,
            &archon_core::config::AnthropicModelsConfig::default(),
        );
        session
            .agent_results
            .push((info, make_agent_result(output)));
    }

    let validator = get_agent_by_key("consistency-validator").unwrap();
    let info = ResearchFacade::to_agent_info(
        validator,
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let (messages, _, _) = facade.build_prompt(&session, &info).await.unwrap();
    let content = messages[0]["content"].as_str().unwrap();

    assert!(content.contains("## Research RLM Identity"));
    assert!(content.contains("## Accepted Output Manifest"));
    assert!(content.contains("## Deterministic Consistency Pre-Scan"));
    assert!(content.contains("Locked chapter count detected: 8"));
    assert!(content.contains("introduction-writer"));
    assert!(content.contains("do not claim that memory"));
}
