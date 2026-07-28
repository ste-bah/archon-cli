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

        if let Some(ca) = coding_agent {
            let md_path = std::path::Path::new(ca.prompt_source_path);
            if md_path.exists()
                && let Ok(content) = std::fs::read_to_string(md_path)
                && let Ok((_frontmatter, body)) = crate::agent_loader::parse_frontmatter(&content)
                && !body.trim().is_empty()
            {
                layers.push(PromptLayer {
                    name: "agent_instructions".to_string(),
                    content: body,
                    priority: TruncationPriority::AgentInstructions,
                    required: false,
                });
            }
        }

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
