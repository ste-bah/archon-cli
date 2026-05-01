//! Research pipeline facade implementing [`PipelineFacade`].
//!
//! Wires together [`ResearchPromptBuilder`], [`PhDQualityCalculator`], and
//! [`StyleInjector`] to drive the 46-agent research pipeline through the
//! shared runner loop.
//!
//! # Memory
//!
//! Per REQ-RESEARCH-008, agent outputs are persisted via `archon-memory`
//! (CozoDB + HNSW) with tags `["phd-pipeline", "<namespace>"]`. LEANN
//! semantic search provides fallback for missing keys.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc::UnboundedSender;

use anyhow::{Context, Result};
use async_trait::async_trait;

use archon_memory::{MemoryTrait, MemoryType, SearchFilter};

use crate::coding::rlm::LeannSearcher;
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

/// Tag applied to all research pipeline memories.
const TAG_PHD_PIPELINE: &str = "phd-pipeline";

// ---------------------------------------------------------------------------
// ResearchFacade
// ---------------------------------------------------------------------------

/// Facade that drives the 46-agent PhD research pipeline.
pub struct ResearchFacade {
    quality_calculator: PhDQualityCalculator,
    prompt_builder: ResearchPromptBuilder,
    /// CozoDB + HNSW memory backend per REQ-RESEARCH-008.
    memory: Arc<dyn MemoryTrait>,
    /// LEANN semantic search fallback for missing memory keys.
    leann_searcher: Option<Arc<dyn LeannSearcher>>,
    /// Project path for memory provenance.
    project_path: String,
    /// Optional style override provided via `--style`.
    style_prompt: Option<String>,
    /// Optional PhD learning integration for recording quality feedback.
    learning: Option<Mutex<PhDLearningIntegration>>,
    /// Optional sender for per-agent progress events (TUI streaming).
    /// Uses internal mutability so the sender can be attached after
    /// construction (it's not known at bootstrap time).
    tui_sender: Mutex<Option<UnboundedSender<String>>>,
}

impl ResearchFacade {
    /// Create a new facade backed by the given memory backend.
    pub fn new(
        memory: Arc<dyn MemoryTrait>,
        leann_searcher: Option<Arc<dyn LeannSearcher>>,
        project_path: String,
        style_prompt: Option<String>,
    ) -> Self {
        Self {
            quality_calculator: PhDQualityCalculator::new(),
            prompt_builder: ResearchPromptBuilder::new(),
            memory,
            leann_searcher,
            project_path,
            style_prompt,
            learning: None,
            tui_sender: Mutex::new(None),
        }
    }

    /// Create a new facade with PhD learning integration enabled.
    pub fn with_learning(
        memory: Arc<dyn MemoryTrait>,
        leann_searcher: Option<Arc<dyn LeannSearcher>>,
        project_path: String,
        style_prompt: Option<String>,
        learning: PhDLearningIntegration,
    ) -> Self {
        Self {
            quality_calculator: PhDQualityCalculator::new(),
            prompt_builder: ResearchPromptBuilder::new(),
            memory,
            leann_searcher,
            project_path,
            style_prompt,
            learning: Some(Mutex::new(learning)),
            tui_sender: Mutex::new(None),
        }
    }

    /// Attach a TUI sender at construction time (builder pattern).
    pub fn with_tui_sender(mut self, tx: UnboundedSender<String>) -> Self {
        self.tui_sender = Mutex::new(Some(tx));
        self
    }

    /// Set the TUI sender after construction (called from dispatch handler).
    pub fn set_tui_sender(&self, tx: UnboundedSender<String>) {
        *self.tui_sender.lock().expect("tui_sender lock") = Some(tx);
    }

    /// Extract the top-level namespace from a memory key for tagging.
    ///
    /// `"research/foundation/framing"` → `"research"`.
    fn key_namespace(key: &str) -> &str {
        key.split('/').next().unwrap_or("research")
    }

    /// Persist a value under the given memory key with `phd-pipeline` tags.
    fn store_memory(&self, key: &str, value: String) {
        let namespace = Self::key_namespace(key);
        let tags: Vec<String> = vec![TAG_PHD_PIPELINE.to_string(), namespace.to_string()];
        let _ = self.memory.store_memory(
            &value,
            key,
            MemoryType::Fact,
            0.5,
            &tags,
            "pipeline",
            &self.project_path,
        );
    }

    /// Recall content for a memory key, with LEANN fallback.
    fn recall_memory(&self, key: &str) -> String {
        // Search by phd-pipeline tag, filter by title match.
        let filter = SearchFilter {
            tags: vec![TAG_PHD_PIPELINE.to_string()],
            ..Default::default()
        };
        if let Ok(memories) = self.memory.search_memories(&filter) {
            for m in &memories {
                if m.title == key {
                    return m.content.clone();
                }
            }
        }

        // LEANN fallback.
        if let Some(ref leann) = self.leann_searcher {
            return leann.search(key);
        }

        String::new()
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
        let tool_access = if agent.phase >= 6 {
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
        // Store output at agent's primary memory key — persisted via
        // CozoDB + HNSW with tags per REQ-RESEARCH-008.
        if let Some(research_agent) = get_agent_by_key(&agent.key)
            && let Some(&primary_key) = research_agent.memory_keys.first()
        {
            self.store_memory(primary_key, result.output.clone());
        }

        // Feed quality to PhD learning subsystem
        if let Some(ref learning_mutex) = self.learning
            && let Ok(mut learning) = learning_mutex.lock()
        {
            learning.record_citation_quality(&agent.key, quality.overall);
        }

        // Emit per-agent progress to TUI if sender is attached.
        if let Some(ref tx) = *self.tui_sender.lock().expect("tui_sender lock") {
            let _ = tx.send(format!(
                "[pipeline phase {}] {} complete (quality: {:.2})\n",
                agent.phase, agent.display_name, quality.overall,
            ));
        }

        Ok(())
    }

    async fn finalize(&self, session: PipelineSession) -> Result<PipelineResult> {
        let total_cost: f64 = session.agent_results.iter().map(|(_, r)| r.cost_usd).sum();
        let duration = session.started_at.elapsed();

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

    // 3. next_agent returns first agent then Done after 46
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

        for i in 0..RESEARCH_AGENTS.len() {
            let agent_info = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[i]);
            let result = make_agent_result("output");
            session.agent_results.push((agent_info, result));
        }

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

    // 5. process_completion stores at memory_keys[0] via MemoryTrait
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

        let stored = facade.recall_memory("research/foundation/framing");
        assert_eq!(stored, "step-back analysis output");
    }

    // 6. Memory flows between agents (store via MemoryTrait, recall in B)
    #[tokio::test]
    async fn memory_flows_between_agents() {
        let facade = make_facade();
        let mut session = facade.init_session("AI research").await.unwrap();

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
        let facade = ResearchFacade::new(
            make_memory(),
            None,
            String::new(),
            Some("Use British English spelling".to_string()),
        );
        let session = facade.init_session("test").await.unwrap();

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
        let facade =
            ResearchFacade::with_learning(make_memory(), None, String::new(), None, learning);
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

        let writer = &RESEARCH_AGENTS[29];
        let writer_info = ResearchFacade::to_agent_info(writer);
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
        let agent_info = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[0]);
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
        let agent_info = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[0]);
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
        let agent_info = ResearchFacade::to_agent_info(&RESEARCH_AGENTS[0]);
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
}
