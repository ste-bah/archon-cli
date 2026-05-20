use super::types::{LearningContext, LearningIntegrationConfig};
use crate::learning::desc::{
    DEFAULT_MAX_INJECTION, DEFAULT_MIN_INJECTION_QUALITY, DescEpisode, DescEpisodeStore,
    InjectionFilter,
};
use crate::learning::gnn::auto_trainer::AutoTrainer;
use crate::learning::reasoning::{ReasoningBank, ReasoningRequest, ReasoningResponse};
use crate::learning::sona::{FeedbackInput, SonaConfig, SonaEngine, Trajectory};
use archon_core::agent::UserCorrectionEventPayload;
use archon_memory::embedding::EmbeddingProvider;
use archon_memory::types::MemoryError;
use cozo::DbInstance;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// LearningIntegration - main orchestrator
// ---------------------------------------------------------------------------

/// Main orchestrator wiring SONA + ReasoningBank into the pipeline.
///
/// All dependencies are optional for graceful degradation - when a subsystem
/// is `None`, the integration simply returns empty/default data for that part.
pub struct LearningIntegration {
    sona: Option<SonaEngine>,
    reasoning_bank: Option<ReasoningBank>,
    desc_store: Option<DescEpisodeStore>,
    config: LearningIntegrationConfig,
    /// Maps agent_name -> active trajectory_id for feedback routing.
    active_trajectories: HashMap<String, String>,
    /// Maps agent_name -> active context for DESC episode persistence.
    active_agent_contexts: HashMap<String, ActiveAgentContext>,
    /// Pipeline session ID for trajectory grouping.
    session_id: String,
    /// GNN auto-trainer hooks (PR 3 v0.1.26). Incremented on memory store and
    /// correction events so the background task can trigger retraining.
    auto_trainer: Option<Arc<AutoTrainer>>,
    /// Governed-learning store used for LearningEvent emission.
    event_store: Option<Arc<DbInstance>>,
}

impl LearningIntegration {
    /// Create a new integration layer. All deps are optional.
    pub fn new(
        sona: Option<SonaEngine>,
        reasoning_bank: Option<ReasoningBank>,
        config: LearningIntegrationConfig,
        auto_trainer: Option<Arc<AutoTrainer>>,
    ) -> Self {
        Self {
            sona,
            reasoning_bank,
            desc_store: None,
            config,
            active_trajectories: HashMap::new(),
            active_agent_contexts: HashMap::new(),
            session_id: uuid::Uuid::new_v4().to_string(),
            auto_trainer,
            event_store: None,
        }
    }

    /// Create a production-persistent SONA integration backed by the same
    /// trajectory store that the GNN trainer queries.
    pub fn new_with_persistent_sona(
        db: Arc<DbInstance>,
        mut config: LearningIntegrationConfig,
        auto_trainer: Option<Arc<AutoTrainer>>,
        gnn_input_dim: usize,
    ) -> Self {
        config.track_trajectories = config.track_trajectories && gnn_input_dim > 0;
        let sona = if config.track_trajectories {
            let sona_config = SonaConfig {
                db: Some(db),
                embedding_provider: Some(Arc::new(DeterministicTrajectoryEmbedding {
                    dim: gnn_input_dim,
                })),
                gnn_input_dim,
                ..SonaConfig::default()
            };
            Some(SonaEngine::new(sona_config))
        } else {
            None
        };

        Self::new(sona, None, config, auto_trainer)
    }

    /// Attach the governed-learning event store used for LearningEvent writes.
    pub fn with_event_store(mut self, event_store: Arc<DbInstance>) -> Self {
        self.event_store = Some(event_store);
        self
    }

    /// Attach a ReasoningBank used for runtime reasoning context injection.
    pub fn with_reasoning_bank(mut self, reasoning_bank: ReasoningBank) -> Self {
        self.reasoning_bank = Some(reasoning_bank);
        self
    }

    /// Attach the DESC episodic memory store used for episode injection and
    /// completion persistence.
    pub fn with_desc_store(mut self, desc_store: DescEpisodeStore) -> Self {
        self.desc_store = Some(desc_store);
        self
    }

    /// Called when an agent starts execution.
    ///
    /// Creates a SONA trajectory (if available) and queries ReasoningBank
    /// for relevant context.
    pub fn on_agent_start(
        &mut self,
        agent_name: &str,
        phase: &str,
        task: &str,
        pipeline_id: &str,
    ) -> LearningContext {
        let mut ctx = LearningContext::default();
        let route = format!("{}{}/{}", self.config.route_prefix, phase, agent_name);
        let session = if pipeline_id.is_empty() {
            self.session_id.clone()
        } else {
            pipeline_id.to_string()
        };

        self.active_agent_contexts.insert(
            agent_name.to_string(),
            ActiveAgentContext {
                phase: phase.to_string(),
                task: task.to_string(),
                session_id: session.clone(),
            },
        );

        if let Some(ref desc_store) = self.desc_store {
            match InjectionFilter::filter_for_injection(
                desc_store,
                phase,
                DEFAULT_MAX_INJECTION,
                DEFAULT_MIN_INJECTION_QUALITY,
            ) {
                Ok(injection) => {
                    ctx.desc_episodes = injection
                        .episodes
                        .into_iter()
                        .map(|ranked| {
                            format!(
                                "episode_id={}, task_type={}, quality={:.2}, outcome={}, summary={}",
                                ranked.episode.episode_id,
                                ranked.episode.task_type,
                                ranked.episode.quality_score,
                                ranked.episode.outcome,
                                ranked.episode.description
                            )
                        })
                        .collect();
                }
                Err(e) => tracing::warn!(agent_name, error = %e, "DESC injection lookup failed"),
            }
        }

        // Create SONA trajectory
        if let Some(ref mut sona) = self.sona
            && self.config.track_trajectories
        {
            let traj: Trajectory = sona.create_trajectory(&route, agent_name, &session);
            ctx.sona_context = format!(
                "trajectory_id={}, route={}, agent={}",
                traj.trajectory_id, traj.route, traj.agent_key
            );
            self.active_trajectories
                .insert(agent_name.to_string(), traj.trajectory_id);
        }

        // Query ReasoningBank for context
        if let Some(ref mut rb) = self.reasoning_bank {
            let request = ReasoningRequest {
                query: task.to_string(),
                query_embedding: None,
                mode: None,
                task_type: None,
                max_results: Some(3),
                confidence_threshold: Some(self.config.quality_threshold),
                context: if ctx.desc_episodes.is_empty() {
                    None
                } else {
                    Some(ctx.desc_episodes.clone())
                },
            };
            let response: ReasoningResponse = rb.reason(&request);
            if response.overall_confidence > 0.0 {
                let patterns: Vec<String> = response
                    .patterns
                    .iter()
                    .map(|p| format!("{} (conf={:.2})", p.template, p.confidence))
                    .collect();
                ctx.reasoning_context = if patterns.is_empty() {
                    format!(
                        "mode={:?}, confidence={:.2}",
                        response.mode_used, response.overall_confidence
                    )
                } else {
                    format!(
                        "mode={:?}, confidence={:.2}, patterns=[{}]",
                        response.mode_used,
                        response.overall_confidence,
                        patterns.join("; ")
                    )
                };
            }
        }

        ctx
    }

    /// Called when an agent completes execution.
    ///
    /// Provides quality feedback to SONA if auto_feedback is enabled.
    pub fn on_agent_complete(
        &mut self,
        agent_name: &str,
        quality_score: f64,
        _output_summary: &str,
    ) {
        if !self.config.auto_feedback {
            return;
        }

        let traj_id = match self.active_trajectories.remove(agent_name) {
            Some(id) => Some(id),
            None => None,
        };

        if let Some(ref mut sona) = self.sona
            && let Some(traj_id) = traj_id.clone()
        {
            let input = FeedbackInput {
                trajectory_id: traj_id,
                quality: quality_score,
                l_score: quality_score, // use quality as l_score proxy
                success_rate: if quality_score >= self.config.quality_threshold {
                    1.0
                } else {
                    quality_score
                },
            };
            // Best-effort feedback - ignore errors
            let _ = sona.provide_feedback(&input);
        }

        if let Some(ref desc_store) = self.desc_store
            && let Some(active) = self.active_agent_contexts.remove(agent_name)
        {
            let episode = DescEpisode {
                episode_id: uuid::Uuid::new_v4().to_string(),
                session_id: active.session_id,
                task_type: active.phase,
                description: active.task,
                solution: truncate_for_episode(_output_summary, 4000),
                outcome: if quality_score >= self.config.quality_threshold {
                    "success".to_string()
                } else {
                    "needs_improvement".to_string()
                },
                quality_score,
                reward: quality_score,
                tags: vec!["pipeline".to_string(), agent_name.to_string()],
                trajectory_id: traj_id,
                created_at: 0,
                updated_at: 0,
            };
            if let Err(e) = desc_store.store_episode(&episode) {
                tracing::warn!(agent_name, error = %e, "DESC episode persistence failed");
            }
        }
    }

    /// Record a new memory for auto-trainer trigger tracking.
    ///
    /// Call this whenever a memory is stored in the pipeline (MemoryGraph, CozoDB, etc.).
    /// The auto-trainer uses this to decide when to retrain.
    pub fn on_memory_stored(&self) {
        if let Some(ref at) = self.auto_trainer {
            at.record_memory();
        }
    }

    /// Record a new correction for auto-trainer trigger tracking.
    ///
    /// Call this whenever a correction feedback event is recorded.
    /// Correction spikes are a strong signal that the GNN needs retraining.
    pub fn on_correction_recorded(&self) {
        if let Some(ref at) = self.auto_trainer {
            at.record_correction();
        }
    }

    /// Emit a UserCorrected LearningEvent into the governed-learning store.
    ///
    /// Called by the agent loop after the inner voice and behavioural-rule
    /// reinforcement paths have already run, so `top_rule_id` reflects the
    /// rule context used for aggregation.
    pub fn record_user_correction_event(&self, payload: UserCorrectionEventPayload) {
        let Some(ref store) = self.event_store else {
            return;
        };

        let rule_id = payload.top_rule_id.unwrap_or_default();
        let signal = serde_json::json!({
            "correction_type": payload.correction_type,
            "user_input_excerpt": payload.user_input_excerpt,
        });

        match archon_learning::events::record_event(
            store.as_ref(),
            &payload.session_context,
            archon_learning::models::LearningEventType::UserCorrected,
            &rule_id,
            None,
            signal,
            1.0,
            "",
        ) {
            Ok(_) => self.persist_user_correction_proposals(store.as_ref(), &rule_id),
            Err(e) => tracing::warn!("record_user_correction_event failed: {e}"),
        }
    }

    fn persist_user_correction_proposals(&self, store: &DbInstance, rule_id: &str) {
        if rule_id.is_empty() {
            return;
        }

        let rule_marker = format!(
            "\"rule_id\":{}",
            serde_json::to_string(rule_id).unwrap_or_else(|_| "\"\"".into())
        );

        let existing = match archon_learning::store::list_behaviour_proposals(store, None) {
            Ok(existing) => existing,
            Err(e) => {
                tracing::warn!("record_user_correction_event proposal lookup failed: {e}");
                return;
            }
        };
        if existing.iter().any(|proposal| {
            proposal.manifest_kind
                == archon_learning::models::BehaviourManifestKind::BehaviouralRuleAdjustment
                && proposal.diff.contains(&rule_marker)
        }) {
            return;
        }

        let events = match archon_learning::store::list_all_learning_events(store) {
            Ok(events) => events,
            Err(e) => {
                tracing::warn!("record_user_correction_event event scan failed: {e}");
                return;
            }
        };

        let proposals =
            match archon_learning::proposal::generate_proposals_for_store(store, &events) {
                Ok(proposals) => proposals,
                Err(e) => {
                    tracing::warn!("record_user_correction_event proposal generation failed: {e}");
                    return;
                }
            };

        for proposal in proposals {
            if proposal.manifest_kind
                != archon_learning::models::BehaviourManifestKind::BehaviouralRuleAdjustment
                || !proposal.diff.contains(&rule_marker)
            {
                continue;
            }
            if let Err(e) = archon_learning::store::insert_behaviour_proposal(store, &proposal) {
                tracing::warn!(
                    proposal_id = %proposal.proposal_id,
                    "record_user_correction_event proposal persist failed: {e}"
                );
                continue;
            }
            match archon_learning::policy::evaluate_proposal(store, &proposal, false, 0) {
                Ok((decision, _)) => {
                    if let Err(e) = archon_learning::apply::apply_decision(
                        store,
                        &proposal.proposal_id,
                        decision,
                        None,
                        Some("learning-integration"),
                    ) {
                        tracing::warn!(
                            proposal_id = %proposal.proposal_id,
                            "record_user_correction_event proposal policy queue failed: {e}"
                        );
                    }
                }
                Err(e) => tracing::warn!(
                    proposal_id = %proposal.proposal_id,
                    "record_user_correction_event proposal policy evaluation failed: {e}"
                ),
            }
        }
    }

    /// Lightweight read-only version of context retrieval.
    ///
    /// Queries ReasoningBank without creating trajectories.
    pub fn get_learning_context(&mut self, task: &str) -> LearningContext {
        let mut ctx = LearningContext::default();

        if let Some(ref mut rb) = self.reasoning_bank {
            let request = ReasoningRequest {
                query: task.to_string(),
                query_embedding: None,
                mode: None,
                task_type: None,
                max_results: Some(3),
                confidence_threshold: Some(self.config.quality_threshold),
                context: None,
            };
            let response = rb.reason(&request);
            if response.overall_confidence > 0.0 {
                ctx.reasoning_context = format!(
                    "mode={:?}, confidence={:.2}",
                    response.mode_used, response.overall_confidence
                );
            }
        }

        ctx
    }
}

struct DeterministicTrajectoryEmbedding {
    dim: usize,
}

#[derive(Debug, Clone)]
struct ActiveAgentContext {
    phase: String,
    task: String,
    session_id: String,
}

fn truncate_for_episode(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

impl EmbeddingProvider for DeterministicTrajectoryEmbedding {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut vector = vec![0.0_f32; self.dim];
                if self.dim == 0 {
                    return vector;
                }
                for token in text.split_whitespace() {
                    let mut hasher = DefaultHasher::new();
                    token.hash(&mut hasher);
                    let hash = hasher.finish();
                    let idx = (hash as usize) % self.dim;
                    vector[idx] += 1.0;
                }
                let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for value in &mut vector {
                        *value /= norm;
                    }
                }
                vector
            })
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.dim
    }
}
