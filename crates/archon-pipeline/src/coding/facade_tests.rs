use super::*;
use crate::coding::agents::Phase;
use crate::runner::AgentResult;
use std::sync::Arc;
use std::time::Duration;

fn make_facade() -> CodingFacade {
    CodingFacade::new()
}

fn make_result(output: &str) -> AgentResult {
    AgentResult {
        output: output.to_string(),
        tool_use_log: Vec::new(),
        tokens_in: 100,
        tokens_out: 50,
        cost_usd: 0.01,
        duration: Duration::from_millis(500),
        quality: None,
    }
}

#[test]
fn facade_implements_pipeline_facade_trait() {
    let facade = make_facade();
    let _boxed: Box<dyn PipelineFacade> = Box::new(facade);
}

#[tokio::test]
async fn init_session_creates_coding_session() {
    let facade = make_facade();
    let session = facade.init_session("build a REST API").await.unwrap();

    assert_eq!(session.pipeline_type, PipelineType::Coding);
    assert_eq!(session.task, "build a REST API");
    assert!(!session.id.is_empty());
    assert!(session.agent_results.is_empty());
}

#[tokio::test]
async fn next_agent_returns_first_agent() {
    let facade = make_facade();
    let session = facade.init_session("some task").await.unwrap();

    match facade.next_agent(&session).await.unwrap() {
        NextAgent::Continue(info) => {
            assert_eq!(info.key, "contract-agent");
            assert_eq!(info.phase, 1);
            assert!(info.critical);
        }
        other => panic!(
            "expected Continue, got {:?}",
            match other {
                NextAgent::Done => "Done",
                NextAgent::Skip(_) => "Skip",
                _ => "Unknown",
            }
        ),
    }
}

#[tokio::test]
async fn next_agent_returns_done_after_all_agents() {
    let facade = make_facade();
    let mut session = facade.init_session("task").await.unwrap();

    for agent in AGENTS.iter() {
        let info = agent_to_info(agent, &AnthropicModelsConfig::default());
        let result = make_result("output");
        session.agent_results.push((info, result));
    }

    assert_eq!(session.agent_results.len(), AGENTS.len());
    match facade.next_agent(&session).await.unwrap() {
        NextAgent::Done => {}
        _ => panic!("expected Done after all agents"),
    }
}

#[tokio::test]
async fn next_agent_groups_parallelizable_same_phase_wave() {
    let facade = make_facade();
    let mut session = facade.init_session("task").await.unwrap();

    for agent in AGENTS
        .iter()
        .take_while(|agent| agent.key != "interface-designer")
    {
        let info = agent_to_info(agent, &AnthropicModelsConfig::default());
        session.agent_results.push((info, make_result("output")));
    }

    match facade.next_agent(&session).await.unwrap() {
        NextAgent::ContinueWave(wave) => {
            let keys = wave
                .iter()
                .map(|agent| agent.key.as_str())
                .collect::<Vec<_>>();
            assert_eq!(keys, vec!["interface-designer", "data-architect"]);
            assert!(wave.iter().all(|agent| agent.parallelizable));
        }
        _ => panic!("expected a deterministic parallel wave"),
    }
}

#[tokio::test]
async fn build_prompt_includes_base_task_and_algorithm() {
    let facade = make_facade();
    let session = facade.init_session("implement parser").await.unwrap();

    let phase4_agent = AGENTS
        .iter()
        .find(|a| a.phase == Phase::Implementation)
        .expect("should have a phase 4 agent");
    let info = agent_to_info(phase4_agent, &AnthropicModelsConfig::default());

    let (messages, system, tools) = facade.build_prompt(&session, &info).await.unwrap();

    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0]["role"], "user");

    let content = messages[0]["content"].as_str().unwrap();
    assert!(
        content.contains(&info.display_name),
        "prompt should contain agent display name"
    );
    assert!(
        content.contains("implement parser"),
        "prompt should contain task"
    );
    assert!(
        content.contains("Algorithm"),
        "prompt should contain algorithm strategy"
    );
    assert!(!system.is_empty());
    assert!(tools.is_empty());
}

#[tokio::test]
async fn inactive_learning_layers_produce_no_errors() {
    let facade = make_facade();
    let session = facade.init_session("any task").await.unwrap();
    let info = agent_to_info(&AGENTS[0], &AnthropicModelsConfig::default());

    let result = facade.build_prompt(&session, &info).await;
    assert!(
        result.is_ok(),
        "build_prompt should not error with inactive learning layers"
    );

    let (messages, _, _) = result.unwrap();
    let content = messages[0]["content"].as_str().unwrap();
    assert!(
        !content.contains("desc_episodes"),
        "empty layers should not appear in prompt"
    );
    assert!(
        !content.contains("sona_patterns"),
        "empty layers should not appear in prompt"
    );
    assert!(
        !content.contains("reflexion_trajectories"),
        "empty layers should not appear in prompt"
    );
    assert!(
        !content.contains("pattern_matcher_results"),
        "empty layers should not appear in prompt"
    );
    assert!(
        !content.contains("sherlock_verdicts"),
        "empty layers should not appear in prompt"
    );
}

#[tokio::test]
async fn leann_context_flows_into_prompt() {
    let facade = make_facade();
    let mut session = facade.init_session("leann test").await.unwrap();
    session.leann_context = "function parse_input at src/parser.rs:42".to_string();

    let info = agent_to_info(&AGENTS[0], &AnthropicModelsConfig::default());
    let (messages, _, _) = facade.build_prompt(&session, &info).await.unwrap();
    let content = messages[0]["content"].as_str().unwrap();

    assert!(
        content.contains("function parse_input at src/parser.rs:42"),
        "LEANN context should appear in prompt when session.leann_context is non-empty"
    );
}

#[tokio::test]
async fn score_quality_returns_valid_score() {
    let facade = make_facade();
    let session = facade.init_session("task").await.unwrap();
    let info = agent_to_info(&AGENTS[0], &AnthropicModelsConfig::default());

    let result = make_result(
        r#"
//! Module documentation
/// Public function
pub fn process(input: &str) -> String {
    input.to_uppercase()
}

pub mod helpers {
    /// Helper
    pub fn noop() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_process() {
        assert_eq!(process("hello"), "HELLO");
    }
}
"#,
    );

    let score = facade
        .score_quality(&session, &info, &result)
        .await
        .unwrap();

    assert!(score.overall >= 0.0 && score.overall <= 1.0);
    assert!(score.dimensions.contains_key("code_quality"));
    assert!(score.dimensions.contains_key("completeness"));
    assert!(score.dimensions.contains_key("structural_integrity"));
    assert!(score.dimensions.contains_key("documentation"));
    assert!(score.dimensions.contains_key("test_coverage"));
    assert!(score.dimensions.contains_key("phase_threshold"));
}

#[tokio::test]
async fn process_completion_writes_to_rlm() {
    let facade = make_facade();
    let mut session = facade.init_session("task").await.unwrap();
    let info = agent_to_info(&AGENTS[0], &AnthropicModelsConfig::default());
    let result = make_result("analysis output");
    let quality = QualityScore {
        overall: 0.9,
        dimensions: HashMap::new(),
    };

    facade
        .process_completion(&mut session, &info, &result, &quality)
        .await
        .unwrap();

    let store = facade.rlm_store.lock().unwrap();
    assert_eq!(
        store.read("coding/understanding/task-analysis"),
        Some("analysis output".to_string()),
    );
    assert_eq!(
        store.read("coding/understanding/parsed-intent"),
        Some("analysis output".to_string()),
    );
}

#[tokio::test]
async fn process_completion_waits_for_progress_capacity() {
    let facade: Arc<dyn PipelineFacade> = Arc::new(make_facade());
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(1);
    progress_tx.send("occupied".to_string()).await.unwrap();
    let progress_facade =
        crate::runner::PipelineProgressFacade::new(Arc::clone(&facade), progress_tx);
    let mut session = facade.init_session("task").await.unwrap();
    let info = agent_to_info(&AGENTS[0], &AnthropicModelsConfig::default());
    let result = make_result("analysis output");
    let quality = QualityScore {
        overall: 0.9,
        dimensions: HashMap::new(),
    };

    let completion = tokio::spawn(async move {
        progress_facade
            .process_completion(&mut session, &info, &result, &quality)
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
        Some("[pipeline phase 1] Contract Agent complete (quality: 0.90)\n")
    );
}

#[tokio::test]
async fn closed_progress_receiver_does_not_fail_completion() {
    let facade: Arc<dyn PipelineFacade> = Arc::new(make_facade());
    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(1);
    drop(progress_rx);
    let progress_facade =
        crate::runner::PipelineProgressFacade::new(Arc::clone(&facade), progress_tx);
    let mut session = facade.init_session("task").await.unwrap();
    let info = agent_to_info(&AGENTS[0], &AnthropicModelsConfig::default());
    let result = make_result("analysis output");
    let quality = QualityScore {
        overall: 0.9,
        dimensions: HashMap::new(),
    };

    progress_facade
        .process_completion(&mut session, &info, &result, &quality)
        .await
        .expect("detached TUI must not fail pipeline completion");
}

#[tokio::test]
async fn build_prompt_is_deterministic() {
    let facade = make_facade();
    let session = facade.init_session("deterministic task").await.unwrap();
    let info = agent_to_info(&AGENTS[0], &AnthropicModelsConfig::default());

    let (msgs1, sys1, tools1) = facade.build_prompt(&session, &info).await.unwrap();
    let (msgs2, sys2, tools2) = facade.build_prompt(&session, &info).await.unwrap();

    assert_eq!(msgs1, msgs2);
    assert_eq!(sys1, sys2);
    assert_eq!(tools1, tools2);
}

#[tokio::test]
async fn score_quality_includes_phase_threshold() {
    let facade = make_facade();
    let session = facade.init_session("task").await.unwrap();

    let info_p1 = agent_to_info(&AGENTS[0], &AnthropicModelsConfig::default());
    let result = make_result("some output");

    let score = facade
        .score_quality(&session, &info_p1, &result)
        .await
        .unwrap();

    let threshold = score.dimensions.get("phase_threshold").unwrap();
    assert_eq!(*threshold, 0.75, "Phase 1 threshold should be 0.75");

    let phase4_agent = AGENTS
        .iter()
        .find(|a| a.phase == Phase::Implementation)
        .unwrap();
    let info_p4 = agent_to_info(phase4_agent, &AnthropicModelsConfig::default());
    let score_p4 = facade
        .score_quality(&session, &info_p4, &result)
        .await
        .unwrap();

    let threshold_p4 = score_p4.dimensions.get("phase_threshold").unwrap();
    assert_eq!(*threshold_p4, 0.85, "Phase 4 threshold should be 0.85");
}

#[test]
fn display_name_conversion() {
    assert_eq!(display_name_from_key("contract-agent"), "Contract Agent");
    assert_eq!(
        display_name_from_key("requirement-extractor"),
        "Requirement Extractor"
    );
    assert_eq!(display_name_from_key("single"), "Single");
}

#[tokio::test]
async fn finalize_produces_pipeline_result() {
    let facade = make_facade();
    let mut session = facade.init_session("finalize task").await.unwrap();
    let session_id = session.id.clone();

    let info1 = agent_to_info(&AGENTS[0], &AnthropicModelsConfig::default());
    let mut result1 = make_result("first output");
    result1.cost_usd = 0.05;
    session.agent_results.push((info1, result1));

    let info2 = agent_to_info(&AGENTS[1], &AnthropicModelsConfig::default());
    let mut result2 = make_result("final output");
    result2.cost_usd = 0.03;
    session.agent_results.push((info2, result2));

    let pipeline_result = facade.finalize(session).await.unwrap();

    assert_eq!(pipeline_result.session_id, session_id);
    assert_eq!(pipeline_result.pipeline_type, PipelineType::Coding);
    assert_eq!(pipeline_result.agent_results.len(), 2);
    assert!((pipeline_result.total_cost_usd - 0.08).abs() < f64::EPSILON);
    assert_eq!(pipeline_result.final_output, "final output");
}

#[tokio::test]
async fn rlm_context_flows_into_prompt() {
    let facade = make_facade();
    let mut session = facade.init_session("flow test").await.unwrap();

    let info = agent_to_info(&AGENTS[0], &AnthropicModelsConfig::default());
    let result = make_result("task analysis: build REST API with auth");
    let quality = QualityScore {
        overall: 0.9,
        dimensions: HashMap::new(),
    };
    facade
        .process_completion(&mut session, &info, &result, &quality)
        .await
        .unwrap();

    let req_agent = agent_to_info(&AGENTS[1], &AnthropicModelsConfig::default());
    let (messages, _, _) = facade.build_prompt(&session, &req_agent).await.unwrap();
    let content = messages[0]["content"].as_str().unwrap();

    assert!(
        content.contains("task analysis: build REST API with auth"),
        "requirement-extractor prompt should contain contract-agent's RLM output"
    );
}

#[test]
fn default_creates_valid_facade() {
    let facade = CodingFacade::default();
    let _boxed: Box<dyn PipelineFacade> = Box::new(facade);
}

#[test]
fn facade_with_learning_implements_trait() {
    use crate::learning::integration::{LearningIntegration, LearningIntegrationConfig};
    let learning = LearningIntegration::new(None, None, LearningIntegrationConfig::default(), None);
    let facade = CodingFacade::with_learning(learning);
    let _boxed: Box<dyn PipelineFacade> = Box::new(facade);
}
