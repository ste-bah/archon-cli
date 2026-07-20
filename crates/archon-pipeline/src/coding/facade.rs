//! CodingFacade — [`PipelineFacade`] implementation for the 50-agent coding
//! pipeline with 11-layer prompt augmentation.
//!
//! Layers L1-L10 are assembled per-agent, then L11 (prompt_cap) enforces the
//! token budget via `PromptBudget`. Layers 5-9 gracefully degrade to empty
//! strings when learning systems are not active.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;

use archon_core::config::{AnthropicModelsConfig, ContextConfig};

use crate::coding::agents::AGENTS;
use crate::coding::quality::{CodingQualityCalculator, phase_threshold};
use crate::coding::rlm::RlmStore;
use crate::learning::integration::LearningIntegration;
use crate::prompt_cap::{PromptBudget, truncate_prompt_to_budget};
use crate::runner::{
    AgentInfo, AgentResult, NextAgent, PipelineFacade, PipelineResult, PipelineSession,
    PipelineType, QualityScore,
};

mod helpers;
mod layers;

use helpers::{
    CODING_PARALLEL_WAVE_LIMIT, agent_to_info, dependencies_satisfied, find_coding_agent,
};
#[cfg(test)]
use helpers::display_name_from_key;

/// Facade implementing the coding pipeline's 50-agent sequence with 11-layer
/// prompt augmentation.
pub struct CodingFacade {
    quality_calculator: CodingQualityCalculator,
    rlm_store: Mutex<RlmStore>,
    learning: Option<Mutex<LearningIntegration>>,
    /// Optional sender for per-agent progress events (TUI streaming).
    /// Uses internal mutability so the sender can be attached after
    /// construction (it's not known at bootstrap time).
    tui_sender: Mutex<Option<UnboundedSender<String>>>,
    /// Anthropic model alias map. Defaults to compile-time defaults; callers
    /// that have an active `ArchonConfig` should pass `config.models.anthropic`
    /// via `with_models(..)` so operator overrides apply.
    models: AnthropicModelsConfig,
    context: ContextConfig,
}

impl CodingFacade {
    /// Create a new facade with an empty RLM store.
    pub fn new() -> Self {
        Self {
            quality_calculator: CodingQualityCalculator::new(),
            rlm_store: Mutex::new(RlmStore::new()),
            learning: None,
            tui_sender: Mutex::new(None),
            models: AnthropicModelsConfig::default(),
            context: ContextConfig::default(),
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
            tui_sender: Mutex::new(None),
            models: AnthropicModelsConfig::default(),
            context: ContextConfig::default(),
        }
    }

    /// Attach a TUI sender at construction time (builder pattern).
    pub fn with_tui_sender(mut self, tx: UnboundedSender<String>) -> Self {
        self.tui_sender = Mutex::new(Some(tx));
        self
    }

    /// Attach an operator-configured Anthropic model alias map (builder pattern).
    ///
    /// When this is not called, the facade uses `AnthropicModelsConfig::default()`
    /// which is the compile-time fallback. Pass `config.models.anthropic.clone()`
    /// from the active `ArchonConfig` to honour operator overrides.
    pub fn with_models(mut self, models: AnthropicModelsConfig) -> Self {
        self.models = models;
        self
    }

    pub fn with_context(mut self, context: ContextConfig) -> Self {
        self.context = context;
        self
    }

    /// Set the TUI sender after construction (called from dispatch handler).
    pub fn set_tui_sender(&self, tx: UnboundedSender<String>) {
        *self.tui_sender.lock().expect("tui_sender lock") = Some(tx);
    }
}

impl Default for CodingFacade {
    fn default() -> Self {
        Self::new()
    }
}

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

    /// Determine the next runnable agent or deterministic parallel wave.
    async fn next_agent(&self, session: &PipelineSession) -> Result<NextAgent> {
        let completed: HashSet<&str> = session
            .agent_results
            .iter()
            .map(|(agent, _)| agent.key.as_str())
            .collect();
        if completed.len() >= AGENTS.len() {
            return Ok(NextAgent::Done);
        }
        let Some(first) = AGENTS.iter().find(|agent| {
            !completed.contains(agent.key) && dependencies_satisfied(agent, &completed)
        }) else {
            anyhow::bail!(
                "coding pipeline cannot find a runnable agent; completed={}",
                completed.len()
            );
        };

        if !first.parallelizable {
            return Ok(NextAgent::Continue(agent_to_info(first, &self.models)));
        }

        let wave: Vec<AgentInfo> = AGENTS
            .iter()
            .filter(|agent| {
                !completed.contains(agent.key)
                    && agent.parallelizable
                    && agent.phase == first.phase
                    && dependencies_satisfied(agent, &completed)
            })
            .take(CODING_PARALLEL_WAVE_LIMIT)
            .map(|agent| agent_to_info(agent, &self.models))
            .collect();

        if wave.len() > 1 {
            Ok(NextAgent::ContinueWave(wave))
        } else {
            Ok(NextAgent::Continue(agent_to_info(first, &self.models)))
        }
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
        self.build_prompt_for_attempt(session, agent, 1).await
    }

    async fn build_prompt_for_attempt(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
        attempt: u8,
    ) -> Result<(
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
        Vec<serde_json::Value>,
    )> {
        let layers = self.build_layers(session, agent)?;
        let context_window = archon_llm::context_window::resolve_context_window(
            &agent.model,
            self.context
                .context_window_override
                .or_else(|| self.context.max_tokens.map(u64::from)),
            None,
        )
        .context_window as usize;
        let budget = PromptBudget::from_context_config(context_window, &self.context, attempt);
        let truncated = truncate_prompt_to_budget(layers, budget.max_prompt_tokens)
            .context("prompt truncation failed")?;

        let assembled = truncated
            .layers
            .iter()
            .map(|layer| layer.content.as_str())
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

        Ok((messages, system, Vec::new()))
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

        if let Some(ref learning_mutex) = self.learning
            && let Ok(mut learning) = learning_mutex.lock()
        {
            learning.on_agent_complete(&agent.key, quality.overall, &result.output);
        }

        if let Some(ref tx) = *self.tui_sender.lock().expect("tui_sender lock") {
            let _ = tx.send(format!(
                "[pipeline phase {}] {} complete (quality: {:.2})\n",
                agent.phase, agent.display_name, quality.overall,
            ));
        }

        Ok(())
    }

    /// Produce the final pipeline result once all agents have finished.
    async fn finalize(&self, session: PipelineSession) -> Result<PipelineResult> {
        let total_cost = session.agent_results.iter().map(|(_, r)| r.cost_usd).sum();
        let duration = session.started_at.elapsed();
        let final_output = session
            .agent_results
            .last()
            .map(|(_, result)| result.output.clone())
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

#[cfg(test)]
#[path = "facade_tests.rs"]
mod tests;
