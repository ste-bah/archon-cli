//! Fail-open integration contracts for runtime consumers.

use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::advisor::{
    WorldAdvisorDecision, WorldAdvisorUnavailable, WorldAdvisorUnavailableReason, WorldPrediction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldAdvisorSurface {
    Pipeline,
    ProviderRuntime,
    MemorySurfacing,
    AgentEvolution,
    InteractiveSession,
    CodingTask,
    ToolRun,
    VerificationRun,
    PipelineStep,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldAdvisorSurfaceRecord {
    pub surface: WorldAdvisorSurface,
    pub prediction: Option<WorldPrediction>,
    pub unavailable: Option<WorldAdvisorUnavailable>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action_summary: Option<String>,
    pub continue_foreground_flow: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldRuntimeOutcomeRecord {
    pub surface: WorldAdvisorSurface,
    pub prediction_id: Option<String>,
    pub session_id: String,
    pub action_ref: String,
    pub actual_summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_class: Option<crate::guardrail::RuntimeTaskClass>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_status: Option<crate::guardrail::GuardrailFinalStatus>,
    #[serde(default)]
    pub verification_outcomes: Vec<crate::guardrail::VerificationOutcome>,
    #[serde(default)]
    pub user_correction_observed: bool,
    #[serde(default)]
    pub plan_drift_observed: bool,
    #[serde(default)]
    pub provider_incident_observed: bool,
    #[serde(default)]
    pub retry_count: u32,
    pub latent_surprise: Option<f32>,
    pub evidence_refs: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldAuditedBundleAttachment {
    pub bundle_id: String,
    pub prediction_id: Option<String>,
    pub outcome_id: String,
    pub evidence_refs: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl WorldAdvisorSurfaceRecord {
    pub fn from_decision(surface: WorldAdvisorSurface, decision: WorldAdvisorDecision) -> Self {
        Self {
            surface,
            prediction: decision.prediction,
            unavailable: decision.unavailable,
            session_id: None,
            action_ref: None,
            action_summary: None,
            continue_foreground_flow: true,
            created_at: Utc::now(),
        }
    }

    pub fn unavailable(
        surface: WorldAdvisorSurface,
        reason: WorldAdvisorUnavailableReason,
    ) -> Self {
        Self {
            surface,
            prediction: None,
            unavailable: Some(WorldAdvisorUnavailable::new(reason)),
            session_id: None,
            action_ref: None,
            action_summary: None,
            continue_foreground_flow: true,
            created_at: Utc::now(),
        }
    }

    pub fn with_context(
        mut self,
        session_id: impl Into<String>,
        action_ref: impl Into<String>,
        action_summary: impl Into<String>,
    ) -> Self {
        self.session_id = Some(session_id.into());
        self.action_ref = Some(action_ref.into());
        self.action_summary = Some(action_summary.into());
        self
    }

    pub fn should_continue(&self) -> bool {
        self.continue_foreground_flow
    }
}

pub fn append_surface_record(
    root: &Path,
    record: &WorldAdvisorSurfaceRecord,
) -> anyhow::Result<PathBuf> {
    let path = root.join("ledgers").join("world-advisor-events.jsonl");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(&line)?;
    Ok(path)
}

pub fn append_runtime_outcome(
    root: &Path,
    record: &WorldRuntimeOutcomeRecord,
) -> anyhow::Result<PathBuf> {
    append_jsonl(root, "world-runtime-outcomes.jsonl", record)
}

pub fn append_bundle_attachment(
    root: &Path,
    record: &WorldAuditedBundleAttachment,
) -> anyhow::Result<PathBuf> {
    append_jsonl(root, "world-bundle-attachments.jsonl", record)
}

fn append_jsonl<T: Serialize>(root: &Path, filename: &str, record: &T) -> anyhow::Result<PathBuf> {
    let path = root.join("ledgers").join(filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?
        .write_all(&line)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn all_runtime_surfaces_fail_open_when_advisor_unavailable() {
        for surface in [
            WorldAdvisorSurface::Pipeline,
            WorldAdvisorSurface::ProviderRuntime,
            WorldAdvisorSurface::MemorySurfacing,
            WorldAdvisorSurface::AgentEvolution,
            WorldAdvisorSurface::InteractiveSession,
            WorldAdvisorSurface::CodingTask,
            WorldAdvisorSurface::ToolRun,
            WorldAdvisorSurface::VerificationRun,
            WorldAdvisorSurface::PipelineStep,
        ] {
            let record = WorldAdvisorSurfaceRecord::unavailable(
                surface,
                WorldAdvisorUnavailableReason::ColdStart,
            );

            assert!(record.prediction.is_none());
            assert!(record.should_continue());
        }
    }

    #[test]
    fn surface_records_append_to_runtime_ledger() {
        let temp = tempfile::tempdir().unwrap();
        let record = WorldAdvisorSurfaceRecord::unavailable(
            WorldAdvisorSurface::Pipeline,
            WorldAdvisorUnavailableReason::ColdStart,
        );

        let path = append_surface_record(temp.path(), &record).unwrap();
        let mut content = String::new();
        std::fs::File::open(path)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();

        assert!(content.contains("\"surface\":\"pipeline\""));
        assert!(content.contains("\"continue_foreground_flow\":true"));
    }

    #[test]
    fn runtime_outcomes_and_bundle_attachments_append_to_ledgers() {
        let temp = tempfile::tempdir().unwrap();
        let outcome = WorldRuntimeOutcomeRecord {
            surface: WorldAdvisorSurface::Pipeline,
            prediction_id: Some("p1".into()),
            session_id: "s1".into(),
            action_ref: "a1".into(),
            actual_summary: "completed".into(),
            task_class: None,
            final_status: None,
            verification_outcomes: Vec::new(),
            user_correction_observed: false,
            plan_drift_observed: false,
            provider_incident_observed: false,
            retry_count: 0,
            latent_surprise: Some(0.2),
            evidence_refs: vec!["bundle:b1".into()],
            created_at: Utc::now(),
        };
        let attachment = WorldAuditedBundleAttachment {
            bundle_id: "b1".into(),
            prediction_id: Some("p1".into()),
            outcome_id: "s1:a1".into(),
            evidence_refs: vec!["prediction:p1".into()],
            created_at: Utc::now(),
        };

        let outcome_path = append_runtime_outcome(temp.path(), &outcome).unwrap();
        let attachment_path = append_bundle_attachment(temp.path(), &attachment).unwrap();

        assert!(
            std::fs::read_to_string(outcome_path)
                .unwrap()
                .contains("completed")
        );
        assert!(
            std::fs::read_to_string(attachment_path)
                .unwrap()
                .contains("\"bundle_id\":\"b1\"")
        );
    }
}
