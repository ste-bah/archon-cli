//! Integration Wiring — orchestrates SONA, ReasoningBank, DESC, and Sherlock
//! learning subsystems into a unified pipeline-facing API.
//!
//! Implements REQ-LEARN-F09.

use std::collections::HashMap;

use super::reasoning::{ReasoningBank, ReasoningRequest, ReasoningResponse};
use super::sona::{FeedbackInput, SonaEngine, Trajectory};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the main learning integration layer.
#[derive(Debug, Clone)]
pub struct LearningIntegrationConfig {
    pub track_trajectories: bool,
    pub auto_feedback: bool,
    pub quality_threshold: f64,
    pub route_prefix: String,
    pub enable_hyperedges: bool,
    pub hyperedge_threshold: f64,
}

impl Default for LearningIntegrationConfig {
    fn default() -> Self {
        Self {
            track_trajectories: true,
            auto_feedback: true,
            quality_threshold: 0.8,
            route_prefix: "pipeline/".to_string(),
            enable_hyperedges: false,
            hyperedge_threshold: 0.85,
        }
    }
}

// ---------------------------------------------------------------------------
// LearningContext
// ---------------------------------------------------------------------------

/// Context returned to the pipeline when an agent starts or queries learning.
#[derive(Debug, Clone, Default)]
pub struct LearningContext {
    pub sona_context: String,
    pub reasoning_context: String,
    pub desc_episodes: Vec<String>,
    pub reflexion: Option<String>,
}

// ---------------------------------------------------------------------------
// LearningIntegration — main orchestrator
// ---------------------------------------------------------------------------

/// Main orchestrator wiring SONA + ReasoningBank into the pipeline.
///
/// All dependencies are optional for graceful degradation — when a subsystem
/// is `None`, the integration simply returns empty/default data for that part.
pub struct LearningIntegration {
    sona: Option<SonaEngine>,
    reasoning_bank: Option<ReasoningBank>,
    config: LearningIntegrationConfig,
    /// Maps agent_name -> active trajectory_id for feedback routing.
    active_trajectories: HashMap<String, String>,
    /// Pipeline session ID for trajectory grouping.
    session_id: String,
}

impl LearningIntegration {
    /// Create a new integration layer. All deps are optional.
    pub fn new(
        sona: Option<SonaEngine>,
        reasoning_bank: Option<ReasoningBank>,
        config: LearningIntegrationConfig,
    ) -> Self {
        Self {
            sona,
            reasoning_bank,
            config,
            active_trajectories: HashMap::new(),
            session_id: uuid::Uuid::new_v4().to_string(),
        }
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

        // Create SONA trajectory
        if let Some(ref mut sona) = self.sona {
            if self.config.track_trajectories {
                let route = format!("{}{}/{}", self.config.route_prefix, phase, agent_name);
                let session = if pipeline_id.is_empty() {
                    &self.session_id
                } else {
                    pipeline_id
                };
                let traj: Trajectory = sona.create_trajectory(&route, agent_name, session);
                ctx.sona_context = format!(
                    "trajectory_id={}, route={}, agent={}",
                    traj.trajectory_id, traj.route, traj.agent_key
                );
                self.active_trajectories
                    .insert(agent_name.to_string(), traj.trajectory_id);
            }
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
            Some(id) => id,
            None => return,
        };

        if let Some(ref mut sona) = self.sona {
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
            // Best-effort feedback — ignore errors
            let _ = sona.provide_feedback(&input);
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

// ---------------------------------------------------------------------------
// PhDLearningIntegration — research-specific
// ---------------------------------------------------------------------------

/// Style feedback entry for a chapter.
#[derive(Debug, Clone)]
pub struct StyleFeedback {
    pub chapter: String,
    pub score: f64,
    pub issues: Vec<String>,
}

/// Research-specific learning integration for PhD pipeline.
///
/// Tracks style consistency feedback per chapter and citation quality scores.
pub struct PhDLearningIntegration {
    style_feedback: Vec<StyleFeedback>,
    citation_scores: Vec<(String, f64)>,
}

impl PhDLearningIntegration {
    pub fn new() -> Self {
        Self {
            style_feedback: Vec::new(),
            citation_scores: Vec::new(),
        }
    }

    /// Record style feedback for a chapter.
    pub fn record_style_feedback(&mut self, chapter: &str, score: f64, issues: Vec<String>) {
        self.style_feedback.push(StyleFeedback {
            chapter: chapter.to_string(),
            score,
            issues,
        });
    }

    /// Record citation quality score for an agent.
    pub fn record_citation_quality(&mut self, agent_name: &str, score: f64) {
        self.citation_scores.push((agent_name.to_string(), score));
    }

    /// Get a summary of style feedback across all chapters.
    pub fn get_style_summary(&self) -> String {
        if self.style_feedback.is_empty() {
            return "No style feedback recorded.".to_string();
        }

        let avg: f64 = self.style_feedback.iter().map(|f| f.score).sum::<f64>()
            / self.style_feedback.len() as f64;

        let all_issues: Vec<&str> = self
            .style_feedback
            .iter()
            .flat_map(|f| f.issues.iter().map(|s| s.as_str()))
            .collect();

        let unique_issues: Vec<&str> = {
            let mut seen = std::collections::HashSet::new();
            all_issues.into_iter().filter(|i| seen.insert(*i)).collect()
        };

        format!(
            "Style avg={:.2}, chapters={}, issues=[{}]",
            avg,
            self.style_feedback.len(),
            unique_issues.join(", ")
        )
    }

    /// Average citation quality across all recorded scores.
    pub fn get_citation_quality_avg(&self) -> f64 {
        if self.citation_scores.is_empty() {
            return 0.0;
        }
        self.citation_scores.iter().map(|(_, s)| s).sum::<f64>() / self.citation_scores.len() as f64
    }
}

impl Default for PhDLearningIntegration {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SherlockLearningIntegration
// ---------------------------------------------------------------------------

/// Sherlock verdict classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SherlockVerdict {
    Approved,
    Rejected,
    NeedsRevision,
}

/// Feeds Sherlock verdicts into the learning subsystems.
pub struct SherlockLearningIntegration {
    sona: Option<SonaEngine>,
    verdicts: Vec<(String, SherlockVerdict)>,
    failed_patterns_list: Vec<String>,
}

impl SherlockLearningIntegration {
    pub fn new(sona: Option<SonaEngine>) -> Self {
        Self {
            sona,
            verdicts: Vec::new(),
            failed_patterns_list: Vec::new(),
        }
    }

    /// Record a Sherlock verdict for an agent.
    ///
    /// Maps Approved->0.9, NeedsRevision->0.5, Rejected->0.2 quality.
    /// If SONA is available and has a trajectory for this agent, records feedback.
    /// Stores failed patterns for Rejected verdicts.
    pub fn record_verdict(&mut self, agent_name: &str, verdict: SherlockVerdict) {
        let quality = match verdict {
            SherlockVerdict::Approved => 0.9,
            SherlockVerdict::NeedsRevision => 0.5,
            SherlockVerdict::Rejected => 0.2,
        };

        self.verdicts.push((agent_name.to_string(), verdict));

        if verdict == SherlockVerdict::Rejected {
            self.failed_patterns_list
                .push(format!("rejected:{}", agent_name));
        }

        // If SONA available, attempt to provide feedback.
        // We create a synthetic trajectory for the verdict since we don't
        // necessarily have an active trajectory ID.
        if let Some(ref mut sona) = self.sona {
            let route = format!("sherlock/{}", agent_name);
            let traj = sona.create_trajectory(&route, agent_name, "sherlock");
            let input = FeedbackInput {
                trajectory_id: traj.trajectory_id,
                quality,
                l_score: quality,
                success_rate: if verdict == SherlockVerdict::Approved {
                    1.0
                } else {
                    quality
                },
            };
            let _ = sona.provide_feedback(&input);
        }
    }

    /// Calculate the pass rate (Approved / total).
    pub fn pass_rate(&self) -> f64 {
        if self.verdicts.is_empty() {
            return 0.0;
        }
        let approved = self
            .verdicts
            .iter()
            .filter(|(_, v)| *v == SherlockVerdict::Approved)
            .count();
        approved as f64 / self.verdicts.len() as f64
    }

    /// Return all recorded failed patterns.
    pub fn failed_patterns(&self) -> &[String] {
        &self.failed_patterns_list
    }
}

// ---------------------------------------------------------------------------
// PipelineMemoryCoordinator
// ---------------------------------------------------------------------------

/// A pending memory store operation.
#[derive(Debug, Clone)]
pub struct PendingStore {
    pub key: String,
    pub value: String,
    pub priority: u32,
}

/// Coordinates memory operations across pipeline agents.
///
/// Queues store operations sorted by priority and provides batch flush.
pub struct PipelineMemoryCoordinator {
    pending: Vec<PendingStore>,
    total_flushes: usize,
}

impl PipelineMemoryCoordinator {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
            total_flushes: 0,
        }
    }

    /// Queue a store operation, keeping the queue sorted by priority (highest first).
    pub fn coordinate_store(&mut self, key: &str, value: &str, priority: u32) {
        let entry = PendingStore {
            key: key.to_string(),
            value: value.to_string(),
            priority,
        };

        // Insert in sorted position (descending by priority).
        let pos = self
            .pending
            .iter()
            .position(|p| p.priority < priority)
            .unwrap_or(self.pending.len());
        self.pending.insert(pos, entry);
    }

    /// Look up a pending store by key.
    pub fn coordinate_recall(&self, key: &str) -> Option<&PendingStore> {
        self.pending.iter().find(|p| p.key == key)
    }

    /// Flush all pending stores, returning them in priority order.
    ///
    /// Clears the internal queue.
    pub fn flush(&mut self) -> Vec<PendingStore> {
        self.total_flushes += 1;
        std::mem::take(&mut self.pending)
    }

    /// Number of pending store operations.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Total number of flush operations performed.
    pub fn total_flushes(&self) -> usize {
        self.total_flushes
    }
}

impl Default for PipelineMemoryCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_agent_start_returns_context_with_all_none() {
        let config = LearningIntegrationConfig::default();
        let mut integration = LearningIntegration::new(None, None, config);

        let ctx = integration.on_agent_start("test-agent", "phase1", "build widget", "pipe-001");

        // Should return default (empty) context without panicking
        assert!(ctx.sona_context.is_empty());
        assert!(ctx.reasoning_context.is_empty());
        assert!(ctx.desc_episodes.is_empty());
        assert!(ctx.reflexion.is_none());
    }

    #[test]
    fn on_agent_complete_works_with_sona_none() {
        let config = LearningIntegrationConfig::default();
        let mut integration = LearningIntegration::new(None, None, config);

        // Should not panic
        integration.on_agent_complete("test-agent", 0.95, "completed successfully");
    }

    #[test]
    fn sherlock_approved_maps_to_high_quality() {
        let mut sherlock = SherlockLearningIntegration::new(None);
        sherlock.record_verdict("agent-a", SherlockVerdict::Approved);

        // Pass rate should be 1.0 with a single approval
        assert!(sherlock.pass_rate() >= 0.8);
    }

    #[test]
    fn sherlock_rejected_maps_to_low_quality() {
        let mut sherlock = SherlockLearningIntegration::new(None);
        sherlock.record_verdict("agent-a", SherlockVerdict::Rejected);

        // Pass rate with a single rejection should be 0.0
        assert!(sherlock.pass_rate() <= 0.3);
        assert!(!sherlock.failed_patterns().is_empty());
    }

    #[test]
    fn sherlock_pass_rate_calculation() {
        let mut sherlock = SherlockLearningIntegration::new(None);
        sherlock.record_verdict("agent-a", SherlockVerdict::Approved);
        sherlock.record_verdict("agent-b", SherlockVerdict::Approved);
        sherlock.record_verdict("agent-c", SherlockVerdict::Rejected);

        let rate = sherlock.pass_rate();
        // 2 approved / 3 total ≈ 0.667
        assert!((rate - 2.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn memory_coordinator_stores_in_priority_order() {
        let mut coord = PipelineMemoryCoordinator::new();
        coord.coordinate_store("low", "val-low", 1);
        coord.coordinate_store("high", "val-high", 10);
        coord.coordinate_store("mid", "val-mid", 5);

        // First item should be highest priority
        let first = coord.coordinate_recall("high");
        assert!(first.is_some());
        assert_eq!(first.unwrap().priority, 10);

        // Verify ordering via flush
        let flushed = coord.flush();
        assert_eq!(flushed.len(), 3);
        assert_eq!(flushed[0].key, "high");
        assert_eq!(flushed[1].key, "mid");
        assert_eq!(flushed[2].key, "low");
    }

    #[test]
    fn memory_coordinator_flush_clears_pending() {
        let mut coord = PipelineMemoryCoordinator::new();
        coord.coordinate_store("k1", "v1", 5);
        coord.coordinate_store("k2", "v2", 3);

        assert_eq!(coord.pending_count(), 2);

        let flushed = coord.flush();
        assert_eq!(flushed.len(), 2);
        assert_eq!(coord.pending_count(), 0);
        assert_eq!(coord.total_flushes(), 1);
    }

    #[test]
    fn phd_citation_quality_average() {
        let mut phd = PhDLearningIntegration::new();
        phd.record_citation_quality("agent-a", 0.8);
        phd.record_citation_quality("agent-b", 0.6);
        phd.record_citation_quality("agent-c", 0.9);

        let avg = phd.get_citation_quality_avg();
        // (0.8 + 0.6 + 0.9) / 3 ≈ 0.7667
        assert!((avg - 0.7667).abs() < 0.01);
    }
}
