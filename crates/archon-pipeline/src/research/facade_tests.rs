use super::*;
use crate::runner::{AgentResult, QualityScore};
use archon_memory::graph::MemoryGraph;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

// -----------------------------------------------------------------------
// Mock LeannSearcher
// -----------------------------------------------------------------------

struct MockLeannSearcher {
    response: String,
    call_count: AtomicUsize,
}

impl MockLeannSearcher {
    fn new(response: &str) -> Self {
        Self {
            response: response.to_string(),
            call_count: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl LeannSearcher for MockLeannSearcher {
    fn search(&self, _query: &str) -> String {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.response.clone()
    }
}

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

fn make_memory() -> Arc<dyn MemoryTrait> {
    Arc::new(MemoryGraph::in_memory().expect("in-memory graph created"))
}

fn make_facade() -> ResearchFacade {
    ResearchFacade::new(make_memory(), None, String::new(), None)
}

fn make_facade_with_leann(response: &str) -> (ResearchFacade, Arc<MockLeannSearcher>) {
    let leann = Arc::new(MockLeannSearcher::new(response));
    let facade = ResearchFacade::new(
        make_memory(),
        Some(leann.clone() as Arc<dyn LeannSearcher>),
        String::new(),
        None,
    );
    (facade, leann)
}

fn make_agent_result(output: &str) -> AgentResult {
    AgentResult {
        output: output.to_string(),
        tool_use_log: Vec::new(),
        tokens_in: 100,
        tokens_out: 200,
        cost_usd: 0.01,
        duration: Duration::from_millis(500),
        quality: None,
    }
}

// 1. ResearchFacade implements PipelineFacade (trait object check)
#[test]
fn facade_implements_pipeline_facade() {
    let facade = make_facade();
    let _: &dyn PipelineFacade = &facade;
}

// 2. init_session creates Research type session
#[tokio::test]
async fn init_session_creates_research_session() {
    let facade = make_facade();
    let session = facade.init_session("test query").await.unwrap();

    assert_eq!(session.pipeline_type, PipelineType::Research);
    assert_eq!(session.task, "test query");
    assert!(!session.id.is_empty());
    assert!(session.agent_results.is_empty());
}

// 3. next_agent returns first agent then Done after all agents
#[tokio::test]
async fn next_agent_sequence() {
    let facade = make_facade();
    let mut session = facade.init_session("test").await.unwrap();

    match facade.next_agent(&session).await.unwrap() {
        NextAgent::Continue(agent) => {
            assert_eq!(agent.key, "step-back-analyzer");
        }
        other => panic!(
            "Expected Continue, got {:?}",
            matches!(other, NextAgent::Done)
        ),
    }

    for agent in RESEARCH_AGENTS {
        let agent_info = ResearchFacade::to_agent_info(
            agent,
            &archon_core::config::AnthropicModelsConfig::default(),
        );
        let result = make_agent_result("output");
        session.agent_results.push((agent_info, result));
    }

    match facade.next_agent(&session).await.unwrap() {
        NextAgent::Done => {}
        _ => panic!("Expected Done after all agents"),
    }
}

#[tokio::test]
async fn next_agent_runs_dynamic_final_stage_steps() {
    let facade = make_facade();
    let mut session = facade.init_session("test").await.unwrap();
    let architecture = "\
**Total Chapters**: 2

### Chapter 1: Introduction
**Expected Word Count**: 3,000 words
**Content Outline**:
- 1.1 Background and problem context
- 1.2 Research aims

### Chapter 2: Methodology
**Expected Word Count**: 3,000 words
**Content Outline**:
- 2.1 Architecture research method
- 2.2 Evaluation approach
";

    for agent in RESEARCH_AGENTS
        .iter()
        .take(final_assembly::STATIC_AGENTS_BEFORE_FINAL)
    {
        let info = ResearchFacade::to_agent_info(
            agent,
            &archon_core::config::AnthropicModelsConfig::default(),
        );
        let output = if info.key == "dissertation-architect" {
            architecture
        } else {
            "accepted output"
        };
        session
            .agent_results
            .push((info, make_agent_result(output)));
    }

    for expected in [
        final_assembly::SCANNER_KEY,
        final_assembly::MAPPER_KEY,
        "chapter-writer-001-introduction",
        "chapter-writer-002-methodology",
        final_assembly::COMBINER_KEY,
        final_assembly::VALIDATOR_KEY,
    ] {
        let NextAgent::Continue(agent) = facade.next_agent(&session).await.unwrap() else {
            panic!("expected final-stage agent {expected}");
        };
        assert_eq!(agent.key, expected);
        session
            .agent_results
            .push((agent, make_agent_result("accepted final-stage output")));
    }

    match facade.next_agent(&session).await.unwrap() {
        NextAgent::Done => {}
        _ => panic!("Expected Done after dynamic final-stage agents"),
    }
}

// 4. score_quality returns valid score
#[tokio::test]
async fn score_quality_returns_valid() {
    let facade = make_facade();
    let session = facade.init_session("test").await.unwrap();
    let agent_info = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[0],
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let result = make_agent_result(
        "## Analysis\n\nThis is a detailed analysis with methodology and framework.\n\
         The theoretical framework suggests important findings.\n\
         Evidence suggests correlation between variables.",
    );

    let score = facade
        .score_quality(&session, &agent_info, &result)
        .await
        .unwrap();

    assert!(score.overall >= 0.0 && score.overall <= 0.95);
    assert!(score.dimensions.contains_key("content_depth"));
    assert!(score.dimensions.contains_key("structural_quality"));
    assert!(score.dimensions.contains_key("research_rigor"));
    assert!(score.dimensions.contains_key("completeness"));
    assert!(score.dimensions.contains_key("format_quality"));
}

#[tokio::test]
async fn citation_reconciler_fail_language_hard_fails_quality() {
    let facade = make_facade();
    let session = facade.init_session("test").await.unwrap();
    let agent = get_agent_by_key("citation-reconciler").unwrap();
    let agent_info = ResearchFacade::to_agent_info(
        agent,
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let result = make_agent_result(
        "# Citation Repair\n\n**Citation Repair Status**: FAIL\n\
         Every in-text citation has reference entry | \u{274c}",
    );

    let score = facade
        .score_quality(&session, &agent_info, &result)
        .await
        .unwrap();

    assert_eq!(score.overall, 0.0);
    assert_eq!(score.dimensions.get("citation_gate"), Some(&0.0));
}

// 5. process_completion stores at memory_keys[0] via MemoryTrait
#[tokio::test]
async fn process_completion_stores_memory() {
    let facade = make_facade();
    let mut session = facade.init_session("test").await.unwrap();
    let agent_info = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[0],
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let result = make_agent_result("step-back analysis output");
    let quality = QualityScore {
        overall: 0.75,
        dimensions: HashMap::new(),
    };

    facade
        .process_completion(&mut session, &agent_info, &result, &quality)
        .await
        .unwrap();

    let stored = facade.recall_memory("research/foundation/framing");
    assert_eq!(stored, "step-back analysis output");
}

#[tokio::test]
async fn process_completion_waits_for_progress_capacity() {
    let facade: Arc<dyn PipelineFacade> = Arc::new(make_facade());
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(1);
    progress_tx.send("occupied".to_string()).await.unwrap();
    let progress_facade =
        crate::runner::PipelineProgressFacade::new(Arc::clone(&facade), progress_tx);
    let mut session = facade.init_session("test").await.unwrap();
    let agent = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[0],
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let result = make_agent_result("step-back analysis output");
    let quality = QualityScore {
        overall: 0.75,
        dimensions: HashMap::new(),
    };

    let completion = tokio::spawn(async move {
        progress_facade
            .process_completion(&mut session, &agent, &result, &quality)
            .await
    });
    tokio::task::yield_now().await;
    assert!(
        !completion.is_finished(),
        "full progress channel did not apply backpressure"
    );

    assert_eq!(progress_rx.recv().await.as_deref(), Some("occupied"));
    completion
        .await
        .expect("completion task")
        .expect("process completion");
    assert_eq!(
        progress_rx.recv().await.as_deref(),
        Some("[pipeline phase 1] Step-Back Analyzer complete (quality: 0.75)\n")
    );
}

#[tokio::test]
async fn closed_progress_receiver_does_not_fail_completion() {
    let facade: Arc<dyn PipelineFacade> = Arc::new(make_facade());
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(1);
    drop(progress_rx);
    let progress_facade =
        crate::runner::PipelineProgressFacade::new(Arc::clone(&facade), progress_tx);
    let mut session = facade.init_session("test").await.unwrap();
    let agent = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[0],
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let result = make_agent_result("step-back analysis output");
    let quality = QualityScore {
        overall: 0.75,
        dimensions: HashMap::new(),
    };

    progress_facade
        .process_completion(&mut session, &agent, &result, &quality)
        .await
        .expect("detached TUI must not fail pipeline completion");
}

// 6. Memory flows between agents (store via MemoryTrait, recall in B)
#[tokio::test]
async fn memory_flows_between_agents() {
    let facade = make_facade();
    let mut session = facade.init_session("AI research").await.unwrap();

    let agent_a = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[0],
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let result_a = make_agent_result("Foundation analysis: AI impacts healthcare deeply.");
    let quality = QualityScore {
        overall: 0.8,
        dimensions: HashMap::new(),
    };
    facade
        .process_completion(&mut session, &agent_a, &result_a, &quality)
        .await
        .unwrap();

    let recalled = facade.recall_memory("research/foundation/framing");
    assert_eq!(
        recalled,
        "Foundation analysis: AI impacts healthcare deeply."
    );
}

// 7. build_prompt returns well-formed triple
#[tokio::test]
async fn build_prompt_returns_triple() {
    let facade = make_facade();
    let session = facade.init_session("test query").await.unwrap();
    let agent_info = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[0],
        &archon_core::config::AnthropicModelsConfig::default(),
    );

    let (messages, system, tools) = facade.build_prompt(&session, &agent_info).await.unwrap();

    assert_eq!(messages.len(), 1);
    assert_eq!(system.len(), 1);
    assert!(tools.is_empty());

    let msg = &messages[0];
    assert_eq!(msg["role"], "user");
    let content = msg["content"].as_str().unwrap();
    assert!(content.contains("## Workflow Context"));
    assert!(content.contains("## Task Completion"));
}

// 8. finalize produces PipelineResult
#[tokio::test]
async fn finalize_produces_result() {
    let facade = make_facade();
    let mut session = facade.init_session("test").await.unwrap();

    let agent_info = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[0],
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    let result = make_agent_result("final output text");
    session.agent_results.push((agent_info, result));

    let pipeline_result = facade.finalize(session).await.unwrap();

    assert_eq!(pipeline_result.pipeline_type, PipelineType::Research);
    assert_eq!(pipeline_result.final_output, "final output text");
    assert_eq!(pipeline_result.agent_results.len(), 1);
}

// 9. Style prompt passed to Phase 6 agents
#[tokio::test]
async fn style_prompt_passed_to_phase6() {
    let facade = ResearchFacade::new(
        make_memory(),
        None,
        String::new(),
        Some("Use British English spelling".to_string()),
    );
    let session = facade.init_session("test").await.unwrap();

    let agent_info = ResearchFacade::to_agent_info(
        &RESEARCH_AGENTS[28],
        &archon_core::config::AnthropicModelsConfig::default(),
    );
    assert_eq!(agent_info.key, "introduction-writer");

    let (messages, _, _) = facade.build_prompt(&session, &agent_info).await.unwrap();
    let content = messages[0]["content"].as_str().unwrap();

    assert!(
        content.contains("## STYLE GUIDELINES"),
        "Phase 6 agent should get style guidelines"
    );
    assert!(
        content.contains("British English"),
        "style content should be injected"
    );
}

#[path = "facade_extended_tests.rs"]
mod extended_tests;
