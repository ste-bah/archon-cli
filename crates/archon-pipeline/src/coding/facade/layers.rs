use anyhow::Result;

use crate::coding::algorithm::select_algorithm;
use crate::prompt_cap::{PromptLayer, TruncationPriority};
use crate::runner::{AgentInfo, PipelineSession};

use super::CodingFacade;
use super::helpers::find_coding_agent;

impl CodingFacade {
    /// Build the 11-layer prompt for a given agent.
    ///
    /// Layers:
    /// - L1  base_prompt: agent metadata and role context
    /// - L2  agent_instructions: required source prompt file
    /// - L3  task_context: the user's task description
    /// - L4  leann_semantic_context: LEANN semantic code context
    /// - L5  rlm_namespace_context: RLM store reads for the agent's memory_reads
    /// - L6  desc_episodes: DESC episodic memory (when learning active)
    /// - L7  sona_patterns: SONA trajectory patterns (when learning active)
    /// - L8  reflexion_trajectories: Reflexion self-correction (when learning active)
    /// - L9  pattern_matcher_results: reasoning context (when learning active)
    /// - L10 sherlock_verdicts: (reserved — wired via SherlockLearningIntegration)
    /// - L11 algorithm_strategy: algorithm prompt snippet
    /// - L12 prompt_cap: token budget enforcement via truncation
    pub(super) fn build_layers(
        &self,
        session: &PipelineSession,
        agent: &AgentInfo,
    ) -> Result<Vec<PromptLayer>> {
        let coding_agent = find_coding_agent(&agent.key);

        let base_prompt = format!(
            "You are the {} agent.\n\nPhase: {}\nModel: {}\n{}",
            agent.display_name,
            agent.phase,
            agent.model,
            coding_agent
                .map(|a| a.description.to_string())
                .unwrap_or_default(),
        );
        let task_context = format!("## Task\n\n{}", session.task);
        let leann_context = session.leann_context.clone();
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
        let algorithm_strategy = coding_agent
            .map(|ca| select_algorithm(ca).prompt_snippet().to_string())
            .unwrap_or_default();

        let mut layers = vec![PromptLayer {
            name: "base_prompt".to_string(),
            content: base_prompt,
            priority: TruncationPriority::Required,
            required: true,
        }];

        let prompt_path = coding_agent
            .map(|ca| ca.prompt_source_path.to_string())
            .unwrap_or_else(|| format!(".archon/agents/coding/{}.md", agent.key));
        let prompt_root = &self.project_root;
        let body = if let Some(coding_agent) = coding_agent {
            crate::coding::load_agent_prompt_body(coding_agent, prompt_root)?
        } else {
            let read_path = prompt_root.join(&prompt_path);
            let content = std::fs::read_to_string(&read_path).map_err(|error| {
                anyhow::anyhow!("Coding pipeline agent '{}' prompt '{}' is unavailable: {error}; run `archon agent sync`.", agent.key, prompt_path)
            })?;
            let (_frontmatter, body) = crate::agent_loader::parse_frontmatter(&content)?;
            if body.trim().is_empty() {
                anyhow::bail!(
                    "Coding pipeline agent '{}' prompt '{}' has an empty body; run `archon agent sync`.",
                    agent.key,
                    prompt_path
                );
            }
            body
        };
        layers.push(PromptLayer {
            name: "agent_instructions".to_string(),
            content: body,
            priority: TruncationPriority::AgentInstructions,
            required: true,
        });

        layers.push(PromptLayer {
            name: "task_context".to_string(),
            content: task_context,
            priority: TruncationPriority::Required,
            required: true,
        });

        if !leann_context.is_empty() {
            layers.push(PromptLayer {
                name: "leann_semantic_context".to_string(),
                content: leann_context,
                priority: TruncationPriority::LeannSemanticContext,
                required: false,
            });
        }
        if !rlm_context.is_empty() {
            layers.push(PromptLayer {
                name: "rlm_namespace_context".to_string(),
                content: rlm_context,
                priority: TruncationPriority::RlmContext,
                required: false,
            });
        }

        if let Some(ref learning_mutex) = self.learning
            && let Ok(mut learning) = learning_mutex.lock()
        {
            let ctx = learning.get_learning_context(&session.task);
            if !ctx.desc_episodes.is_empty() {
                layers.push(PromptLayer {
                    name: "desc_episodes".to_string(),
                    content: ctx.desc_episodes.join("\n\n"),
                    priority: TruncationPriority::DescEpisodes,
                    required: false,
                });
            }
            if !ctx.sona_context.is_empty() {
                layers.push(PromptLayer {
                    name: "sona_patterns".to_string(),
                    content: ctx.sona_context,
                    priority: TruncationPriority::SonaPatterns,
                    required: false,
                });
            }
            if let Some(ref reflexion) = ctx.reflexion
                && !reflexion.is_empty()
            {
                layers.push(PromptLayer {
                    name: "reflexion_trajectories".to_string(),
                    content: reflexion.clone(),
                    priority: TruncationPriority::ReflexionTrajectories,
                    required: false,
                });
            }
            if !ctx.reasoning_context.is_empty() {
                layers.push(PromptLayer {
                    name: "pattern_matcher_results".to_string(),
                    content: ctx.reasoning_context,
                    priority: TruncationPriority::PatternMatcherResults,
                    required: false,
                });
            }
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn missing_runtime_prompt_fails_layer_build_with_sync_guidance() {
        let facade = CodingFacade::new();
        let session = PipelineSession {
            id: "prompt-test".into(),
            pipeline_type: crate::runner::PipelineType::Coding,
            task: "task".into(),
            started_at: Instant::now(),
            agent_results: Vec::new(),
            leann_context: String::new(),
        };
        let agent = AgentInfo {
            key: "missing-runtime-prompt-for-test".into(),
            display_name: "Test Agent".into(),
            model: "test".into(),
            phase: 1,
            critical: false,
            parallelizable: false,
            quality_threshold: 0.0,
            tool_access_level: crate::runner::ToolAccessLevel::ReadOnly,
        };
        let error = facade
            .build_layers(&session, &agent)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Coding pipeline"));
        assert!(error.contains(&agent.key));
        assert!(error.contains(".archon/agents/coding/missing-runtime-prompt-for-test.md"));
        assert!(error.contains("archon agent sync"));
    }
}
