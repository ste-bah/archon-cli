use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCompletionStatus {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentPerformanceEvent {
    pub event_id: String,
    pub agent_type: String,
    pub agent_version: Option<String>,
    pub run_id: Option<String>,
    pub pipeline_id: Option<String>,
    pub phase: Option<String>,
    pub task_hash: Option<String>,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    pub profile_id: Option<String>,
    pub permission_mode: Option<String>,
    pub completion_status: AgentCompletionStatus,
    pub applied_rate: Option<f64>,
    pub completion_rate: Option<f64>,
    pub quality_score: Option<f64>,
    pub l_score: Option<f64>,
    pub user_accepted: Option<bool>,
    pub user_corrected: Option<bool>,
    pub gate_failed: Option<String>,
    pub test_failed: bool,
    pub provider_incident_id: Option<String>,
    pub evidence_ids: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl AgentPerformanceEvent {
    pub fn new(agent_type: impl Into<String>) -> Self {
        Self {
            event_id: agent_performance_event_id(),
            agent_type: agent_type.into(),
            agent_version: None,
            run_id: None,
            pipeline_id: None,
            phase: None,
            task_hash: None,
            model_id: None,
            provider_id: None,
            profile_id: None,
            permission_mode: None,
            completion_status: AgentCompletionStatus::Inconclusive,
            applied_rate: None,
            completion_rate: None,
            quality_score: None,
            l_score: None,
            user_accepted: None,
            user_corrected: None,
            gate_failed: None,
            test_failed: false,
            provider_incident_id: None,
            evidence_ids: Vec::new(),
            created_at: Utc::now(),
        }
    }

    pub fn with_agent_version(mut self, version: impl Into<String>) -> Self {
        self.agent_version = Some(version.into());
        self
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn with_pipeline_id(mut self, pipeline_id: impl Into<String>) -> Self {
        self.pipeline_id = Some(pipeline_id.into());
        self
    }

    pub fn with_phase(mut self, phase: impl Into<String>) -> Self {
        self.phase = Some(phase.into());
        self
    }

    pub fn with_task_hash(mut self, task_hash: impl Into<String>) -> Self {
        self.task_hash = Some(task_hash.into());
        self
    }

    pub fn with_model_provider(
        mut self,
        model_id: impl Into<String>,
        provider_id: impl Into<String>,
    ) -> Self {
        self.model_id = Some(model_id.into());
        self.provider_id = Some(provider_id.into());
        self
    }

    pub fn with_profile(mut self, profile_id: impl Into<String>) -> Self {
        self.profile_id = Some(profile_id.into());
        self
    }

    pub fn with_permission_mode(mut self, permission_mode: impl Into<String>) -> Self {
        self.permission_mode = Some(permission_mode.into());
        self
    }

    pub fn with_completion_status(mut self, status: AgentCompletionStatus) -> Self {
        self.completion_status = status;
        self
    }

    pub fn with_scores(
        mut self,
        applied_rate: Option<f64>,
        completion_rate: Option<f64>,
        quality_score: Option<f64>,
        l_score: Option<f64>,
    ) -> Self {
        self.applied_rate = applied_rate.map(clamp_unit);
        self.completion_rate = completion_rate.map(clamp_unit);
        self.quality_score = quality_score.map(clamp_unit);
        self.l_score = l_score.map(clamp_unit);
        self
    }

    pub fn with_user_feedback(mut self, accepted: Option<bool>, corrected: Option<bool>) -> Self {
        self.user_accepted = accepted;
        self.user_corrected = corrected;
        self
    }

    pub fn with_gate_failed(mut self, gate_failed: impl Into<String>) -> Self {
        self.gate_failed = Some(gate_failed.into());
        self
    }

    pub fn with_test_failed(mut self, test_failed: bool) -> Self {
        self.test_failed = test_failed;
        self
    }

    pub fn with_provider_incident(mut self, provider_incident_id: impl Into<String>) -> Self {
        self.provider_incident_id = Some(provider_incident_id.into());
        self
    }

    pub fn add_evidence(mut self, evidence_id: impl Into<String>) -> Self {
        let evidence_id = evidence_id.into();
        if !self
            .evidence_ids
            .iter()
            .any(|existing| existing == &evidence_id)
        {
            self.evidence_ids.push(evidence_id);
        }
        self
    }

    pub fn is_positive_signal(&self) -> bool {
        self.completion_status == AgentCompletionStatus::Succeeded
            && self.user_corrected != Some(true)
            && !self.test_failed
            && self.gate_failed.is_none()
            && self.quality_score.unwrap_or(0.0) >= 0.75
    }

    pub fn is_negative_signal(&self) -> bool {
        self.completion_status == AgentCompletionStatus::Failed
            || self.user_corrected == Some(true)
            || self.test_failed
            || self.gate_failed.is_some()
            || self.provider_incident_id.is_some()
    }
}

pub fn agent_performance_event_id() -> String {
    format!("agent-ledger-{}", uuid::Uuid::new_v4())
}

fn clamp_unit(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_event_serializes_core_prd_fields() {
        let event = AgentPerformanceEvent::new("code-reviewer")
            .with_agent_version("1.2.0")
            .with_run_id("run-1")
            .with_pipeline_id("pipe-1")
            .with_phase("review")
            .with_task_hash("task-hash")
            .with_model_provider("claude-sonnet-4-6", "anthropic")
            .with_profile("oauth-primary")
            .with_permission_mode("default")
            .with_completion_status(AgentCompletionStatus::Succeeded)
            .with_scores(Some(1.2), Some(0.8), Some(0.9), Some(-1.0))
            .add_evidence("ev-1")
            .add_evidence("ev-1");

        let value = serde_json::to_value(&event).unwrap();

        assert_eq!(value["agent_type"], "code-reviewer");
        assert_eq!(value["completion_status"], "succeeded");
        assert_eq!(event.applied_rate, Some(1.0));
        assert_eq!(event.l_score, Some(0.0));
        assert_eq!(event.evidence_ids, vec!["ev-1"]);
        assert!(event.event_id.starts_with("agent-ledger-"));
    }

    #[test]
    fn positive_signal_requires_success_quality_and_no_correction() {
        let positive = AgentPerformanceEvent::new("planner")
            .with_completion_status(AgentCompletionStatus::Succeeded)
            .with_scores(None, None, Some(0.9), None);
        let corrected = positive.clone().with_user_feedback(Some(false), Some(true));

        assert!(positive.is_positive_signal());
        assert!(!corrected.is_positive_signal());
        assert!(corrected.is_negative_signal());
    }

    #[test]
    fn provider_incident_and_gate_failure_are_negative_signals() {
        let incident = AgentPerformanceEvent::new("coder").with_provider_incident("prov-1");
        let gate_failed = AgentPerformanceEvent::new("coder").with_gate_failed("tests");

        assert!(incident.is_negative_signal());
        assert!(gate_failed.is_negative_signal());
    }
}
