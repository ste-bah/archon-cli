//! An outcome that already happened, recorded after the fact.
use super::*;

/// One finished agent outcome a caller records after the fact, such as a
/// workflow fold replaying a run's records.
pub struct RecordedOutcome<'a> {
    /// The DESC episode's key. The same outcome recorded again under the
    /// same key replaces its episode instead of adding another.
    pub episode_id: &'a str,
    pub agent: &'a str,
    pub phase: &'a str,
    pub task: &'a str,
    /// The run or pipeline the outcome belongs to; empty means this session.
    pub session_id: &'a str,
    pub quality: f64,
    pub summary: &'a str,
}

impl LearningIntegration {
    /// Record `outcome` as `on_agent_start` then `on_agent_complete` would,
    /// without the lookups only a starting agent uses: no DESC injection
    /// query and no ReasoningBank query, because no agent receives that
    /// context. The DESC episode is keyed by `outcome.episode_id`, so a
    /// repeated delivery is idempotent.
    ///
    /// `Ok(true)` when a DESC episode was written, `Ok(false)` when nothing
    /// persists an episode here (no DESC store, or feedback disabled), and
    /// the store's error when the write failed.
    pub fn record_outcome(&mut self, outcome: &RecordedOutcome<'_>) -> anyhow::Result<bool> {
        if !self.config.auto_feedback {
            return Ok(false);
        }
        let session = if outcome.session_id.is_empty() {
            self.session_id.clone()
        } else {
            outcome.session_id.to_string()
        };
        let route = format!(
            "{}{}/{}",
            self.config.route_prefix, outcome.phase, outcome.agent
        );
        let mut trajectory_id = None;
        if let Some(ref mut sona) = self.sona
            && self.config.track_trajectories
        {
            let trajectory: Trajectory = sona.create_trajectory(&route, outcome.agent, &session);
            let input = FeedbackInput {
                trajectory_id: trajectory.trajectory_id.clone(),
                quality: outcome.quality,
                l_score: outcome.quality,
                success_rate: if outcome.quality >= self.config.quality_threshold {
                    1.0
                } else {
                    outcome.quality
                },
            };
            // Best-effort, as in `on_agent_complete`.
            let _ = sona.provide_feedback(&input);
            trajectory_id = Some(trajectory.trajectory_id);
        }
        let Some(ref desc_store) = self.desc_store else {
            return Ok(false);
        };
        let episode = DescEpisode {
            episode_id: outcome.episode_id.to_string(),
            session_id: session,
            task_type: outcome.phase.to_string(),
            description: outcome.task.to_string(),
            solution: truncate_for_episode(outcome.summary, 4000),
            outcome: if outcome.quality >= self.config.quality_threshold {
                "success".to_string()
            } else {
                "needs_improvement".to_string()
            },
            quality_score: outcome.quality,
            reward: outcome.quality,
            tags: vec!["pipeline".to_string(), outcome.agent.to_string()],
            trajectory_id,
            created_at: 0,
            updated_at: 0,
        };
        desc_store.store_episode(&episode).map(|_| true)
    }
}
