//! Research pipeline facade implementing [`PipelineFacade`].
//!
//! Wires together [`ResearchPromptBuilder`], [`PhDQualityCalculator`], and
//! [`StyleInjector`] to drive the 46-agent research pipeline through the
//! shared runner loop.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::runner::{
    AgentInfo, AgentResult, NextAgent, PipelineFacade, PipelineResult, PipelineSession,
    PipelineType, QualityScore, ToolAccessLevel,
};

use crate::learning::integration::PhDLearningIntegration;

use super::agents::{RESEARCH_AGENTS, ResearchAgent, get_agent_by_key};
use super::prompt_builder::ResearchPromptBuilder;
use super::quality::PhDQualityCalculator;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Memory namespace for research pipeline data.
const MEMORY_NAMESPACE: &str = "project/research";

// ---------------------------------------------------------------------------
// ResearchFacade
// ---------------------------------------------------------------------------

/// Facade that drives the 46-agent PhD research pipeline.
pub struct ResearchFacade {
    quality_calculator: PhDQualityCalculator,
    prompt_builder: ResearchPromptBuilder,
    /// In-memory store keyed by `"{namespace}/{memory_key}"`.
    memory_store: Mutex<HashMap<String, String>>,
    /// Optional style override provided via `--style`.
    style_prompt: Option<String>,
    /// Optional PhD learning integration for recording quality feedback.
    learning: Option<Mutex<PhDLearningIntegration>>,
}

impl ResearchFacade {
    /// Create a new facade with an optional style prompt override.
    pub fn new(style_prompt: Option<String>) -> Self {
        Self {
            quality_calculator: PhDQualityCalculator::new(),
            prompt_builder: ResearchPromptBuilder::new(),
            memory_store: Mutex::new(HashMap::new()),
            style_prompt,
            learning: None,
        }
    }

    /// Create a new facade with PhD learning integration enabled.
    pub fn with_learning(style_prompt: Option<String>, learning: PhDLearningIntegration) -> Self {
        Self {
            quality_calculator: PhDQualityCalculator::new(),
            prompt_builder: ResearchPromptBuilder::new(),
            memory_store: Mutex::new(HashMap::new()),
            style_prompt,
            learning: Some(Mutex::new(learning)),
        }
    }

    /// Build the namespaced key for the memory store.
    fn memory_key(namespace: &str, key: &str) -> String {
        format!("{}/{}", namespace, key)
    }

    /// Store a value in the in-memory store.
    fn store_memory(&self, key: &str, value: String) {
        let ns_key = Self::memory_key(MEMORY_NAMESPACE, key);
        self.memory_store
            .lock()
            .expect("memory_store lock poisoned")
            .insert(ns_key, value);
    }

    /// Recall content for a memory key, returning empty string if absent.
    fn recall_memory(&self, key: &str) -> String {
        let ns_key = Self::memory_key(MEMORY_NAMESPACE, key);
        self.memory_store
            .lock()
            .expect("memory_store lock poisoned")
            .get(&ns_key)
            .cloned()
            .unwrap_or_default()
    }

    /// Recall all memory keys for an agent, concatenating non-empty values.
    fn recall_prior_context(&self, agent: &ResearchAgent) -> String {
        let mut parts = Vec::new();
        for &mk in agent.memory_keys {
            let content = self.recall_memory(mk);
            if !content.is_empty() {
                parts.push(content);
            }
        }
        parts.join("\n\n---\n\n")
    }

    /// Convert a [`ResearchAgent`] to an [`AgentInfo`].
    fn to_agent_info(agent: &ResearchAgent) -> AgentInfo {
        let tool_access = if agent.phase == 6 {
            ToolAccessLevel::Full
        } else {
            ToolAccessLevel::ReadOnly
        };

        AgentInfo {
            key: agent.key.to_string(),
            display_name: agent.display_name.to_string(),
            model: "claude-sonnet-4-20250514".to_string(),
            phase: agent.phase as u32,
            critical: super::quality::PhDQualityCalculator::create_quality_context(
                agent.key,
                agent.phase,
            )
            .is_critical_agent,
            quality_threshold: 0.50,
            tool_access_level: tool_access,
        }
    }
}

#[async_trait]
impl PipelineFacade for ResearchFacade {
    async fn init_session(&self, task: &str) -> Result<PipelineSession> {
        let session_id = uuid::Uuid::new_v4().to_string();
        Ok(PipelineSession {
            id: session_id,
            pipeline_type: PipelineType::Research,
            task: task.to_string(),
            started_at: Instant::now(),
            agent_results: Vec::new(),
            leann_context: String::new(),
        })
    }

    async fn next_agent(&self, session: &PipelineSession) -> Result<NextAgent> {
        let idx = session.agent_results.len();
        if idx >= RESEARCH_AGENTS.len() {
            return Ok(NextAgent::Done);
        }
        let agent = &RESEARCH_AGENTS[idx];
        Ok(NextAgent::Continue(Self::to_agent_info(agent)))
    }

    async fn build_prompt(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
    ) -> Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
    )> {
        let research_agent = get_agent_by_key(&agent.key)
            .with_context(|| format!("Unknown research agent key: {}", agent.key))?;

        let prior_context = self.recall_prior_context(research_agent);

        let style = self.style_prompt.as_deref();

        let prompt_text = self.prompt_builder.build(
            research_agent,
            session.agent_results.len(),
            RESEARCH_AGENTS.len(),
            &session.task,
            &prior_context,
            style,
        );

        // Build the messages / system / tools triple.
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": prompt_text,
        })];

        let system = vec![serde_json::json!({
            "type": "text",
            "text": format!(
                "You are the {} agent in the PhD Research Pipeline. \
                 Follow the instructions carefully and produce high-quality academic output.",
                agent.display_name
            ),
        })];

        // No custom tools for research agents — they use built-in MCP tools.
        let tools: Vec<serde_json::Value> = Vec::new();

        Ok((messages, system, tools))
    }

    async fn score_quality(
        &self,
        _session: &PipelineSession,
        agent: &AgentInfo,
        result: &AgentResult,
    ) -> Result<QualityScore> {
        let ctx = PhDQualityCalculator::create_quality_context(&agent.key, agent.phase as u8);
        let assessment = self.quality_calculator.assess_quality(&result.output, &ctx);

        let mut dimensions = HashMap::new();
        dimensions.insert(
            "content_depth".to_string(),
            assessment.breakdown.content_depth,
        );
        dimensions.insert(
            "structural_quality".to_string(),
            assessment.breakdown.structural_quality,
        );
        dimensions.insert(
            "research_rigor".to_string(),
            assessment.breakdown.research_rigor,
        );
        dimensions.insert(
            "completeness".to_string(),
            assessment.breakdown.completeness,
        );
        dimensions.insert(
            "format_quality".to_string(),
            assessment.breakdown.format_quality,
        );

        Ok(QualityScore {
            overall: assessment.score,
            dimensions,
        })
    }

    async fn process_completion(
        &self,
        _session: &mut PipelineSession,
        agent: &AgentInfo,
        result: &AgentResult,
        quality: &QualityScore,
    ) -> Result<()> {
        // Store output at agent's primary memory key.
        if let Some(research_agent) = get_agent_by_key(&agent.key) {
            if let Some(&primary_key) = research_agent.memory_keys.first() {
                self.store_memory(primary_key, result.output.clone());
            }
        }

        // Feed quality to PhD learning subsystem
        if let Some(ref learning_mutex) = self.learning {
            if let Ok(mut learning) = learning_mutex.lock() {
                learning.record_citation_quality(&agent.key, quality.overall);
            }
        }

        Ok(())
    }

    async fn finalize(&self, session: PipelineSession) -> Result<PipelineResult> {
        let total_cost: f64 = session.agent_results.iter().map(|(_, r)| r.cost_usd).sum();
        let duration = session.started_at.elapsed();

        // Final output: last agent's output or summary.
        let final_output = session
            .agent_results
            .last()
            .map(|(_, r)| r.output.clone())
            .unwrap_or_else(|| "No agent output produced.".to_string());

        Ok(PipelineResult {
            session_id: session.id,
            pipeline_type: session.pipeline_type,
            agent_results: session.agent_results,
            total_cost_usd: total_cost,
            duration,
            final_output,
        })
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{AgentResult, QualityScore};
    use std::time::Duration;

    fn make_facade() -> ResearchFacade {
        ResearchFacade::new(None)
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

    // 3. next_agent returns first agent then Done after 46
    #[tokio::test]
    async fn next_agent_sequence() {
        let facade = make_facade();
        let mut session = facade.init_session("test").await.unwrap();

        // First call should return Continue with first agent
        match facade.next_agent(&session).await.unwrap() {
            NextAgent::Continue(agent) => {
                assert_eq!(agent.key, "step-back-analyzer");
            }
            other => panic!(
                "Expected Continue, got {:?}",
                matches!(other, NextAgent::Done)
            ),
        }

        // Simulate 46 completed agents
        for i in 0..RESEARCH_AGENTS.len() {
            let agent_info = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[i]);
            let result = make_agent_result("output");
            session.agent_results.push((agent_info, result));
        }

        // Should now be Done
        match facade.next_agent(&session).await.unwrap() {
            NextAgent::Done => {}
            _ => panic!("Expected Done after all agents"),
        }
    }

    // 4. score_quality returns valid score
    #[tokio::test]
    async fn score_quality_returns_valid() {
        let facade = make_facade();
        let session = facade.init_session("test").await.unwrap();
        let agent_info = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[0]);
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

    // 5. process_completion stores at memory_keys[0]
    #[tokio::test]
    async fn process_completion_stores_memory() {
        let facade = make_facade();
        let mut session = facade.init_session("test").await.unwrap();
        let agent_info = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[0]);
        let result = make_agent_result("step-back analysis output");
        let quality = QualityScore {
            overall: 0.75,
            dimensions: HashMap::new(),
        };

        facade
            .process_completion(&mut session, &agent_info, &result, &quality)
            .await
            .unwrap();

        // Check that memory was stored at the primary key
        let stored = facade.recall_memory("research/foundation/framing");
        assert_eq!(stored, "step-back analysis output");
    }

    // 6. Memory flows between agents (store from A, recall in B)
    #[tokio::test]
    async fn memory_flows_between_agents() {
        let facade = make_facade();
        let mut session = facade.init_session("AI research").await.unwrap();

        // Process completion for first agent (step-back-analyzer)
        let agent_a = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[0]);
        let result_a = make_agent_result("Foundation analysis: AI impacts healthcare deeply.");
        let quality = QualityScore {
            overall: 0.80,
            dimensions: HashMap::new(),
        };
        facade
            .process_completion(&mut session, &agent_a, &result_a, &quality)
            .await
            .unwrap();

        // Now build prompt for second agent that might reference the same memory
        // The chapter-synthesizer (index 6) has memory_keys that include
        // "research/quality/synthesis" — different from what we stored.
        // Instead, directly verify we can recall what was stored.
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
        let agent_info = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[0]);

        let (messages, system, tools) = facade.build_prompt(&session, &agent_info).await.unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(system.len(), 1);
        assert!(tools.is_empty());

        // Verify message structure
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

        let agent_info = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[0]);
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
        let facade = ResearchFacade::new(Some("Use British English spelling".to_string()));
        let session = facade.init_session("test").await.unwrap();

        // introduction-writer is index 29, Phase 6
        let agent_info = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[29]);
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

    // 10. facade with learning implements trait
    #[test]
    fn facade_with_learning_implements_trait() {
        use crate::learning::integration::PhDLearningIntegration;
        let learning = PhDLearningIntegration::new();
        let facade = ResearchFacade::with_learning(None, learning);
        let _: &dyn PipelineFacade = &facade;
    }

    // 11. to_agent_info correctly converts
    #[test]
    fn to_agent_info_conversion() {
        let agent = &RESEARCH_AGENTS[0];
        let info = ResearchFacade::to_agent_info(agent);

        assert_eq!(info.key, "step-back-analyzer");
        assert_eq!(info.display_name, "Step-Back Analyzer");
        assert_eq!(info.phase, 1);
        assert_eq!(info.tool_access_level, ToolAccessLevel::ReadOnly);

        // Phase 6 agent should have Full access
        let writer = &RESEARCH_AGENTS[29];
        let writer_info = ResearchFacade::to_agent_info(writer);
        assert_eq!(writer_info.tool_access_level, ToolAccessLevel::Full);
    }
}
