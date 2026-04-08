//! CodingFacade — [`PipelineFacade`] implementation for the 48-agent coding
//! pipeline with 11-layer prompt augmentation.
//!
//! Layers L1-L10 are assembled per-agent, then L11 (prompt_cap) enforces the
//! token budget via [`truncate_prompt`]. Layers 5-9 gracefully degrade to empty
//! strings when learning systems are not active.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::coding::agents::{AGENTS, CodingAgent, ToolAccess};
use crate::coding::algorithm::select_algorithm;
use crate::coding::quality::{CodingQualityCalculator, phase_threshold};
use crate::coding::rlm::RlmStore;
use crate::learning::integration::LearningIntegration;
use crate::prompt_cap::{PromptLayer, TruncationPriority, truncate_prompt};
use crate::runner::{
    AgentInfo, AgentResult, NextAgent, PipelineFacade, PipelineResult, PipelineSession,
    PipelineType, QualityScore, ToolAccessLevel,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Claude's context window size in tokens.
const MODEL_CONTEXT_WINDOW: usize = 200_000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a kebab-case key like `"contract-agent"` into a title-case display
/// name like `"Task Analyzer"`.
fn display_name_from_key(key: &str) -> String {
    key.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => {
                    let upper: String = c.to_uppercase().collect();
                    format!("{}{}", upper, chars.collect::<String>())
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Convert a [`CodingAgent`] to an [`AgentInfo`] for the runner.
fn agent_to_info(agent: &CodingAgent) -> AgentInfo {
    AgentInfo {
        key: agent.key.to_string(),
        display_name: display_name_from_key(agent.key),
        model: agent.model.to_string(),
        phase: agent.phase as u32,
        critical: agent.critical,
        quality_threshold: phase_threshold(agent.phase as u32),
        tool_access_level: match agent.tool_access {
            ToolAccess::ReadOnly => ToolAccessLevel::ReadOnly,
            ToolAccess::Full => ToolAccessLevel::Full,
        },
    }
}

/// Find a [`CodingAgent`] in the static `AGENTS` array by key.
fn find_coding_agent(key: &str) -> Option<&'static CodingAgent> {
    AGENTS.iter().find(|a| a.key == key)
}

// ---------------------------------------------------------------------------
// CodingFacade
// ---------------------------------------------------------------------------

/// Facade implementing the coding pipeline's 48-agent sequence with 11-layer
/// prompt augmentation.
pub struct CodingFacade {
    quality_calculator: CodingQualityCalculator,
    rlm_store: Mutex<RlmStore>,
    learning: Option<Mutex<LearningIntegration>>,
}

impl CodingFacade {
    /// Create a new facade with an empty RLM store.
    pub fn new() -> Self {
        Self {
            quality_calculator: CodingQualityCalculator::new(),
            rlm_store: Mutex::new(RlmStore::new()),
            learning: None,
        }
    }

    /// Create a new facade wired to a [`LearningIntegration`] instance.
    ///
    /// Layers L5-L9 will be populated from the learning subsystem when
    /// context is available.
    pub fn with_learning(learning: LearningIntegration) -> Self {
        Self {
            quality_calculator: CodingQualityCalculator::new(),
            rlm_store: Mutex::new(RlmStore::new()),
            learning: Some(Mutex::new(learning)),
        }
    }

    /// Build the 11-layer prompt for a given agent.
    ///
    /// Layers:
    /// - L1  base_prompt: agent-specific system prompt (fallback if file missing)
    /// - L2  task_context: the user's task description
    /// - L3  leann_semantic_context: LEANN semantic code context
    /// - L4  rlm_namespace_context: RLM store reads for the agent's memory_reads
    /// - L5  desc_episodes: DESC episodic memory (when learning active)
    /// - L6  sona_patterns: SONA trajectory patterns (when learning active)
    /// - L7  reflexion_trajectories: Reflexion self-correction (when learning active)
    /// - L8  pattern_matcher_results: reasoning context (when learning active)
    /// - L9  sherlock_verdicts: (reserved — wired via SherlockLearningIntegration)
    /// - L10 algorithm_strategy: algorithm prompt snippet
    /// - L11 prompt_cap: token budget enforcement via truncation
    fn build_layers(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
    ) -> Result<Vec<PromptLayer>> {
        let coding_agent = find_coding_agent(&agent.key);

        // L1: base prompt — fallback to generated prompt
        let base_prompt = format!(
            "You are the {} agent.\n\nPhase: {}\nModel: {}\n{}",
            agent.display_name,
            agent.phase,
            agent.model,
            coding_agent
                .map(|a| a.description.to_string())
                .unwrap_or_default(),
        );

        // L2: task context
        let task_context = format!("## Task\n\n{}", session.task);

        // L3: LEANN semantic context
        let leann_context = session.leann_context.clone();

        // L4: RLM namespace context
        let rlm_context = if let Some(ca) = coding_agent {
            let store = self
                .rlm_store
                .lock()
                .map_err(|e| anyhow::anyhow!("RLM store lock poisoned: {}", e))?;
            let mut parts = Vec::new();
            for ns in ca.memory_reads {
                if let Some(content) = store.read(ns) {
                    parts.push(format!("### {}\n\n{}", ns, content));
                }
            }
            parts.join("\n\n")
        } else {
            String::new()
        };

        // L10: algorithm strategy
        let algorithm_strategy = coding_agent
            .map(|ca| select_algorithm(ca).prompt_snippet().to_string())
            .unwrap_or_default();

        // Assemble layers — only include non-empty optional layers
        let mut layers = Vec::new();

        // L1 (required)
        layers.push(PromptLayer {
            name: "base_prompt".to_string(),
            content: base_prompt,
            priority: TruncationPriority::Required,
            required: true,
        });

        // L2 (required)
        layers.push(PromptLayer {
            name: "task_context".to_string(),
            content: task_context,
            priority: TruncationPriority::Required,
            required: true,
        });

        // L3 (optional — only if non-empty)
        if !leann_context.is_empty() {
            layers.push(PromptLayer {
                name: "leann_semantic_context".to_string(),
                content: leann_context,
                priority: TruncationPriority::LeannSemanticContext,
                required: false,
            });
        }

        // L4 (optional — only if non-empty)
        if !rlm_context.is_empty() {
            layers.push(PromptLayer {
                name: "rlm_namespace_context".to_string(),
                content: rlm_context,
                priority: TruncationPriority::RlmContext,
                required: false,
            });
        }

        // L5-L9: learning system layers (graceful degradation when None)
        if let Some(ref learning_mutex) = self.learning {
            if let Ok(mut learning) = learning_mutex.lock() {
                let ctx = learning.get_learning_context(&session.task);
                // L5: DESC episodes
                if !ctx.desc_episodes.is_empty() {
                    let desc_text = ctx.desc_episodes.join("\n\n");
                    layers.push(PromptLayer {
                        name: "desc_episodes".to_string(),
                        content: desc_text,
                        priority: TruncationPriority::DescEpisodes,
                        required: false,
                    });
                }
                // L6: SONA patterns
                if !ctx.sona_context.is_empty() {
                    layers.push(PromptLayer {
                        name: "sona_patterns".to_string(),
                        content: ctx.sona_context,
                        priority: TruncationPriority::SonaPatterns,
                        required: false,
                    });
                }
                // L7: Reflexion trajectories
                if let Some(ref reflexion) = ctx.reflexion {
                    if !reflexion.is_empty() {
                        layers.push(PromptLayer {
                            name: "reflexion_trajectories".to_string(),
                            content: reflexion.clone(),
                            priority: TruncationPriority::ReflexionTrajectories,
                            required: false,
                        });
                    }
                }
                // L8: Reasoning context as pattern matcher results
                if !ctx.reasoning_context.is_empty() {
                    layers.push(PromptLayer {
                        name: "pattern_matcher_results".to_string(),
                        content: ctx.reasoning_context,
                        priority: TruncationPriority::PatternMatcherResults,
                        required: false,
                    });
                }
            }
        }

        // L10 (optional — only if non-empty)
        if !algorithm_strategy.is_empty() {
            layers.push(PromptLayer {
                name: "algorithm_strategy".to_string(),
                content: algorithm_strategy,
                priority: TruncationPriority::AlgorithmStrategy,
                required: false,
            });
        }

        Ok(layers)
    }
}

impl Default for CodingFacade {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PipelineFacade implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl PipelineFacade for CodingFacade {
    /// Create a fresh session for the given task description.
    async fn init_session(&self, task: &str) -> Result<PipelineSession> {
        let session_id = uuid::Uuid::new_v4().to_string();
        Ok(PipelineSession {
            id: session_id,
            pipeline_type: PipelineType::Coding,
            task: task.to_string(),
            started_at: Instant::now(),
            agent_results: Vec::new(),
            leann_context: String::new(),
        })
    }

    /// Determine the next agent by using the count of completed results as
    /// the index into the static `AGENTS` array.
    async fn next_agent(&self, session: &PipelineSession) -> Result<NextAgent> {
        let idx = session.agent_results.len();
        if idx >= AGENTS.len() {
            return Ok(NextAgent::Done);
        }
        let coding_agent = &AGENTS[idx];
        Ok(NextAgent::Continue(agent_to_info(coding_agent)))
    }

    /// Build the (messages, system, tools) triple with 11-layer prompt
    /// augmentation and token budget enforcement.
    async fn build_prompt(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
    ) -> Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
    )> {
        // Build layers L1-L10
        let layers = self.build_layers(session, agent)?;

        // L11: apply prompt_cap truncation
        let truncated =
            truncate_prompt(layers, MODEL_CONTEXT_WINDOW).context("prompt truncation failed")?;

        // Assemble the final prompt from surviving layers
        let assembled: String = truncated
            .layers
            .iter()
            .map(|l| l.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        let messages = vec![serde_json::json!({
            "role": "user",
            "content": assembled,
        })];

        let system = vec![serde_json::json!({
            "type": "text",
            "text": format!(
                "You are a pipeline agent in the Archon coding pipeline. Agent: {}. Phase: {}.",
                agent.display_name,
                agent.phase,
            ),
        })];

        let tools: Vec<serde_json::Value> = Vec::new();

        Ok((messages, system, tools))
    }

    /// Score the quality of an agent's output using the coding quality calculator.
    async fn score_quality(
        &self,
        _session: &PipelineSession,
        agent: &AgentInfo,
        result: &AgentResult,
    ) -> Result<QualityScore> {
        let breakdown = self.quality_calculator.score(&result.output);

        let mut dimensions = HashMap::new();
        dimensions.insert("code_quality".to_string(), breakdown.code_quality);
        dimensions.insert("completeness".to_string(), breakdown.completeness);
        dimensions.insert(
            "structural_integrity".to_string(),
            breakdown.structural_integrity,
        );
        dimensions.insert("documentation".to_string(), breakdown.documentation);
        dimensions.insert("test_coverage".to_string(), breakdown.test_coverage);
        dimensions.insert("phase_threshold".to_string(), phase_threshold(agent.phase));

        Ok(QualityScore {
            overall: breakdown.composite,
            dimensions,
        })
    }

    /// Write agent output to RLM store at the agent's memory_writes namespaces.
    async fn process_completion(
        &self,
        _session: &mut PipelineSession,
        agent: &AgentInfo,
        result: &AgentResult,
        quality: &QualityScore,
    ) -> Result<()> {
        if let Some(coding_agent) = find_coding_agent(&agent.key) {
            let mut store = self
                .rlm_store
                .lock()
                .map_err(|e| anyhow::anyhow!("RLM store lock poisoned: {}", e))?;
            for ns in coding_agent.memory_writes {
                store.write(ns, &result.output);
            }
        }

        // Feed quality score to learning subsystem
        if let Some(ref learning_mutex) = self.learning {
            if let Ok(mut learning) = learning_mutex.lock() {
                learning.on_agent_complete(&agent.key, quality.overall, &result.output);
            }
        }

        Ok(())
    }

    /// Produce the final pipeline result once all agents have finished.
    async fn finalize(&self, session: PipelineSession) -> Result<PipelineResult> {
        let total_cost: f64 = session.agent_results.iter().map(|(_, r)| r.cost_usd).sum();

        let duration = session.started_at.elapsed();

        let final_output = session
            .agent_results
            .last()
            .map(|(_, r)| r.output.clone())
            .unwrap_or_default();

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
    use crate::coding::agents::Phase;
    use crate::runner::AgentResult;
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

    // -----------------------------------------------------------------------
    // 1. CodingFacade implements PipelineFacade (compile-time check)
    // -----------------------------------------------------------------------

    #[test]
    fn facade_implements_pipeline_facade_trait() {
        let facade = make_facade();
        // If this compiles, the trait is implemented.
        let _boxed: Box<dyn PipelineFacade> = Box::new(facade);
    }

    // -----------------------------------------------------------------------
    // 2. init_session creates session with Coding type and correct task
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn init_session_creates_coding_session() {
        let facade = make_facade();
        let session = facade.init_session("build a REST API").await.unwrap();

        assert_eq!(session.pipeline_type, PipelineType::Coding);
        assert_eq!(session.task, "build a REST API");
        assert!(!session.id.is_empty());
        assert!(session.agent_results.is_empty());
    }

    // -----------------------------------------------------------------------
    // 3. next_agent returns first agent (contract-agent) then increments
    // -----------------------------------------------------------------------

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

    // -----------------------------------------------------------------------
    // 4. next_agent returns Done after all 48 agents
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn next_agent_returns_done_after_all_agents() {
        let facade = make_facade();
        let mut session = facade.init_session("task").await.unwrap();

        // Fill session with 48 fake results
        for agent in AGENTS.iter() {
            let info = agent_to_info(agent);
            let result = make_result("output");
            session.agent_results.push((info, result));
        }

        assert_eq!(session.agent_results.len(), AGENTS.len());
        match facade.next_agent(&session).await.unwrap() {
            NextAgent::Done => {} // expected
            _ => panic!("expected Done after all agents"),
        }
    }

    // -----------------------------------------------------------------------
    // 5. build_prompt for a Phase 4 agent includes L1, L2, L10 content
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn build_prompt_includes_base_task_and_algorithm() {
        let facade = make_facade();
        let session = facade.init_session("implement parser").await.unwrap();

        // Use the first Phase 4 agent
        let phase4_agent = AGENTS
            .iter()
            .find(|a| a.phase == Phase::Implementation)
            .expect("should have a phase 4 agent");
        let info = agent_to_info(phase4_agent);

        let (messages, system, tools) = facade.build_prompt(&session, &info).await.unwrap();

        // messages should have one user message
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");

        let content = messages[0]["content"].as_str().unwrap();

        // L1: base prompt with agent name
        assert!(
            content.contains(&info.display_name),
            "prompt should contain agent display name"
        );

        // L2: task context
        assert!(
            content.contains("implement parser"),
            "prompt should contain task"
        );

        // L10: algorithm snippet — Phase 4 uses SelfDebug
        assert!(
            content.contains("Algorithm"),
            "prompt should contain algorithm strategy"
        );

        // system message present
        assert!(!system.is_empty());

        // tools empty for now
        assert!(tools.is_empty());
    }

    // -----------------------------------------------------------------------
    // 6. Layers 5-9 are empty when learning systems inactive — no errors
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn inactive_learning_layers_produce_no_errors() {
        let facade = make_facade();
        let session = facade.init_session("any task").await.unwrap();
        let info = agent_to_info(&AGENTS[0]);

        // This should succeed without errors even though layers 5-9 are inactive
        let result = facade.build_prompt(&session, &info).await;
        assert!(
            result.is_ok(),
            "build_prompt should not error with inactive learning layers"
        );

        // Verify the prompt does NOT contain learning system markers
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

    // -----------------------------------------------------------------------
    // 6b. LEANN context flows into prompt when session.leann_context is set
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn leann_context_flows_into_prompt() {
        let facade = make_facade();
        let mut session = facade.init_session("leann test").await.unwrap();
        // Inject LEANN context into session
        session.leann_context = "function parse_input at src/parser.rs:42".to_string();

        let info = agent_to_info(&AGENTS[0]);
        let (messages, _, _) = facade.build_prompt(&session, &info).await.unwrap();
        let content = messages[0]["content"].as_str().unwrap();

        assert!(
            content.contains("function parse_input at src/parser.rs:42"),
            "LEANN context should appear in prompt when session.leann_context is non-empty"
        );
    }

    // -----------------------------------------------------------------------
    // 7. score_quality returns valid QualityScore with overall and dimensions
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn score_quality_returns_valid_score() {
        let facade = make_facade();
        let session = facade.init_session("task").await.unwrap();
        let info = agent_to_info(&AGENTS[0]);

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

    // -----------------------------------------------------------------------
    // 8. process_completion writes to RLM store
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn process_completion_writes_to_rlm() {
        let facade = make_facade();
        let mut session = facade.init_session("task").await.unwrap();
        let info = agent_to_info(&AGENTS[0]); // contract-agent
        let result = make_result("analysis output");
        let quality = QualityScore {
            overall: 0.9,
            dimensions: HashMap::new(),
        };

        facade
            .process_completion(&mut session, &info, &result, &quality)
            .await
            .unwrap();

        // contract-agent writes to coding/understanding/task-analysis and
        // coding/understanding/parsed-intent
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

    // -----------------------------------------------------------------------
    // 9. build_prompt is deterministic given same inputs
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn build_prompt_is_deterministic() {
        let facade = make_facade();
        let session = facade.init_session("deterministic task").await.unwrap();
        let info = agent_to_info(&AGENTS[0]);

        let (msgs1, sys1, tools1) = facade.build_prompt(&session, &info).await.unwrap();
        let (msgs2, sys2, tools2) = facade.build_prompt(&session, &info).await.unwrap();

        assert_eq!(msgs1, msgs2);
        assert_eq!(sys1, sys2);
        assert_eq!(tools1, tools2);
    }

    // -----------------------------------------------------------------------
    // 10. Phase threshold applied during quality scoring
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn score_quality_includes_phase_threshold() {
        let facade = make_facade();
        let session = facade.init_session("task").await.unwrap();

        // Phase 1 agent (Understanding) -> threshold 0.75
        let info_p1 = agent_to_info(&AGENTS[0]);
        let result = make_result("some output");

        let score = facade
            .score_quality(&session, &info_p1, &result)
            .await
            .unwrap();

        let threshold = score.dimensions.get("phase_threshold").unwrap();
        assert_eq!(*threshold, 0.75, "Phase 1 threshold should be 0.75");

        // Find a Phase 4 agent for comparison
        let phase4_agent = AGENTS
            .iter()
            .find(|a| a.phase == Phase::Implementation)
            .unwrap();
        let info_p4 = agent_to_info(phase4_agent);
        let score_p4 = facade
            .score_quality(&session, &info_p4, &result)
            .await
            .unwrap();

        let threshold_p4 = score_p4.dimensions.get("phase_threshold").unwrap();
        assert_eq!(*threshold_p4, 0.85, "Phase 4 threshold should be 0.85");
    }

    // -----------------------------------------------------------------------
    // 11. display_name_from_key conversion
    // -----------------------------------------------------------------------

    #[test]
    fn display_name_conversion() {
        assert_eq!(display_name_from_key("contract-agent"), "Contract Agent");
        assert_eq!(
            display_name_from_key("requirement-extractor"),
            "Requirement Extractor"
        );
        assert_eq!(display_name_from_key("single"), "Single");
    }

    // -----------------------------------------------------------------------
    // 12. finalize produces correct PipelineResult
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn finalize_produces_pipeline_result() {
        let facade = make_facade();
        let mut session = facade.init_session("finalize task").await.unwrap();
        let session_id = session.id.clone();

        // Add two fake results
        let info1 = agent_to_info(&AGENTS[0]);
        let mut result1 = make_result("first output");
        result1.cost_usd = 0.05;
        session.agent_results.push((info1, result1));

        let info2 = agent_to_info(&AGENTS[1]);
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

    // -----------------------------------------------------------------------
    // 13. RLM context flows into subsequent agent prompts
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn rlm_context_flows_into_prompt() {
        let facade = make_facade();
        let mut session = facade.init_session("flow test").await.unwrap();

        // Simulate contract-agent completion which writes to RLM
        let info = agent_to_info(&AGENTS[0]);
        let result = make_result("task analysis: build REST API with auth");
        let quality = QualityScore {
            overall: 0.9,
            dimensions: HashMap::new(),
        };
        facade
            .process_completion(&mut session, &info, &result, &quality)
            .await
            .unwrap();

        // Now build prompt for requirement-extractor (reads coding/understanding/task-analysis)
        let req_agent = agent_to_info(&AGENTS[1]);
        let (messages, _, _) = facade.build_prompt(&session, &req_agent).await.unwrap();
        let content = messages[0]["content"].as_str().unwrap();

        assert!(
            content.contains("task analysis: build REST API with auth"),
            "requirement-extractor prompt should contain contract-agent's RLM output"
        );
    }

    // -----------------------------------------------------------------------
    // 14. Default trait implementation
    // -----------------------------------------------------------------------

    #[test]
    fn default_creates_valid_facade() {
        let facade = CodingFacade::default();
        let _boxed: Box<dyn PipelineFacade> = Box::new(facade);
    }

    // -----------------------------------------------------------------------
    // 15. with_learning constructor creates valid facade
    // -----------------------------------------------------------------------

    #[test]
    fn facade_with_learning_implements_trait() {
        use crate::learning::integration::{LearningIntegration, LearningIntegrationConfig};
        let learning = LearningIntegration::new(None, None, LearningIntegrationConfig::default());
        let facade = CodingFacade::with_learning(learning);
        let _boxed: Box<dyn PipelineFacade> = Box::new(facade);
    }
}
