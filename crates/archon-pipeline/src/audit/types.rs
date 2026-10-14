use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::runner::{PipelineType, QualityScore, ToolAccessLevel, ToolUseEntry};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleManifest {
    pub schema_version: u32,
    pub session_id: String,
    pub pipeline_type: PipelineType,
    pub archon_version: String,
    pub worktree_path: String,
    pub initial_git_head: Option<String>,
    pub initial_worktree_dirty: Option<bool>,
    pub task: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum BundleStatus {
    Running,
    Completed,
    Failed,
    Aborted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleState {
    pub session_id: String,
    pub pipeline_type: PipelineType,
    pub task: String,
    pub status: BundleStatus,
    pub current_agent_key: Option<String>,
    pub completed_agent_count: usize,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    /// Unknown usage counts are tracked separately so known sums are never presented as complete.
    #[serde(default)]
    pub tokens_in_unknown_agents: u64,
    #[serde(default)]
    pub tokens_out_unknown_agents: u64,
    pub total_cost_usd: f64,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub final_output_hash: Option<String>,
    #[serde(default)]
    pub completion_integrity_summary: Option<String>,
    #[serde(default)]
    pub completion_report_id: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptAuditRecord {
    pub ordinal: usize,
    pub agent_key: String,
    pub messages_hash: String,
    pub system_hash: String,
    pub tools_hash: String,
    pub messages: Vec<serde_json::Value>,
    pub system: Vec<serde_json::Value>,
    pub tools: Vec<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// New binaries read older audit records; older binaries are not required to read new null values.
/// Legacy numeric zeroes remain recorded as zero because they cannot be distinguished from measured zeroes.
pub struct AgentAttemptRecord {
    pub attempt: usize,
    #[serde(default)]
    pub output_path: Option<String>,
    pub output_hash: String,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub duration_ms: u64,
    pub quality: Option<QualityScore>,
    pub accepted: bool,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// New binaries read older audit records; older binaries are not required to read new null values.
/// Legacy numeric zeroes remain recorded as zero because they cannot be distinguished from measured zeroes.
pub struct AgentAuditRecord {
    pub ordinal: usize,
    pub agent_key: String,
    pub display_name: String,
    pub phase: u32,
    pub requested_model: String,
    pub critical: bool,
    pub quality_threshold: f64,
    pub tool_access_level: ToolAccessLevel,
    pub prompt_record_path: String,
    pub prompt_hash: String,
    pub system_hash: String,
    pub tools_hash: String,
    pub output_path: String,
    pub output_hash: String,
    pub tokens_in: Option<u64>,
    pub tokens_out: Option<u64>,
    pub cost_usd: f64,
    pub duration_ms: u64,
    pub quality: Option<QualityScore>,
    pub tool_use_log: Vec<ToolUseEntry>,
    pub attempts: Vec<AgentAttemptRecord>,
    pub completed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PipelineEvent {
    RunCreated {
        session_id: String,
        pipeline_type: PipelineType,
    },
    RunResumed {
        completed_agent_count: usize,
    },
    RunRewound {
        from_completed_agent_count: usize,
        to_completed_agent_count: usize,
        reason: String,
    },
    AgentPlanned {
        ordinal: usize,
        agent_key: String,
        phase: u32,
    },
    PromptBuilt {
        ordinal: usize,
        agent_key: String,
        messages_hash: String,
        system_hash: String,
        tools_hash: String,
    },
    LlmAttemptStarted {
        ordinal: usize,
        agent_key: String,
        attempt: usize,
        model: String,
    },
    LlmAttemptCompleted {
        ordinal: usize,
        agent_key: String,
        attempt: usize,
        output_hash: String,
        tokens_in: Option<u64>,
        tokens_out: Option<u64>,
        duration_ms: u64,
    },
    LlmAttemptFailed {
        ordinal: usize,
        agent_key: String,
        attempt: usize,
        error: String,
    },
    QualityScored {
        ordinal: usize,
        agent_key: String,
        attempt: usize,
        overall: f64,
        threshold: f64,
        accepted: bool,
    },
    QualityGateForceAccepted {
        ordinal: usize,
        agent_key: String,
        attempt: usize,
        overall: f64,
        threshold: f64,
        reason: String,
    },
    AgentRetried {
        ordinal: usize,
        agent_key: String,
        attempt: usize,
        reason: String,
    },
    AgentCompleted {
        ordinal: usize,
        agent_key: String,
        output_hash: String,
    },
    CompletionChecked {
        final_state: String,
        claim_count: usize,
        verified_claim_count: usize,
        report_id: String,
    },
    ArtifactWritten {
        artifact_type: String,
        path: String,
        content_hash: String,
    },
    RunCompleted {
        final_output_hash: String,
        completed_agent_count: usize,
    },
    RunFailed {
        error: String,
    },
    RunAborted {
        reason: String,
    },
}

#[cfg(test)]
mod usage_serialization_tests {
    use super::{AgentAttemptRecord, AgentAuditRecord, PipelineEvent};

    fn assert_legacy_attempt_fields(value: serde_json::Value) {
        let record: AgentAttemptRecord = serde_json::from_value(value).expect("legacy attempt");
        assert_eq!(record.tokens_in, Some(12));
        assert_eq!(record.tokens_out, Some(3));
    }

    fn assert_legacy_audit_fields(value: serde_json::Value) {
        let record: AgentAuditRecord = serde_json::from_value(value).expect("legacy audit");
        assert_eq!(record.tokens_in, Some(12));
        assert_eq!(record.tokens_out, Some(3));
    }

    #[test]
    fn unknown_attempt_usage_round_trips_as_null() {
        let record: AgentAttemptRecord = serde_json::from_value(serde_json::json!({
            "attempt": 1,
            "output_hash": "hash",
            "tokens_in": null,
            "tokens_out": null,
            "duration_ms": 1,
            "quality": null,
            "accepted": true,
            "failure_reason": null,
            "created_at": "2026-10-14T00:00:00Z"
        }))
        .expect("unknown counts deserialize");

        let value = serde_json::to_value(record).expect("record serializes");
        assert!(value["tokens_in"].is_null());
        assert!(value["tokens_out"].is_null());

        let reported_zero = serde_json::from_value::<AgentAttemptRecord>(serde_json::json!({
            "attempt": 1,
            "output_hash": "hash",
            "tokens_in": 0,
            "tokens_out": 0,
            "duration_ms": 1,
            "quality": null,
            "accepted": true,
            "failure_reason": null,
            "created_at": "2026-10-14T00:00:00Z"
        }))
        .expect("reported zero deserializes");
        assert_eq!(reported_zero.tokens_in, Some(0));
        assert_eq!(reported_zero.tokens_out, Some(0));
        let value = serde_json::to_value(reported_zero).expect("record serializes");
        assert_eq!(value["tokens_in"], 0);
        assert_eq!(value["tokens_out"], 0);
        eprintln!(
            "EVIDENCE #369 legacy zero ACTION: deserialize and re-read numeric zero counts; EXPECTED/ACTUAL: tokens_in={}, tokens_out={} remain recorded zero; SOURCE: serialized audit attempt",
            value["tokens_in"], value["tokens_out"]
        );
    }

    #[test]
    fn legacy_attempt_records_with_numeric_or_missing_usage_deserialize() {
        let base = serde_json::json!({
            "attempt": 1,
            "output_hash": "hash",
            "duration_ms": 1,
            "quality": null,
            "accepted": true,
            "failure_reason": null,
            "created_at": "2026-10-14T00:00:00Z"
        });
        let mut numeric = base.clone();
        numeric["tokens_in"] = serde_json::json!(12);
        numeric["tokens_out"] = serde_json::json!(3);
        assert_legacy_attempt_fields(numeric);

        let missing: AgentAttemptRecord = serde_json::from_value(base).expect("missing usage");
        assert_eq!(missing.tokens_in, None);
        assert_eq!(missing.tokens_out, None);
        println!(
            "EVIDENCE #369 audit-attempt compatibility BEFORE: legacy numeric and absent token fields; AFTER: numeric=(Some(12), Some(3)), absent=({:?}, {:?})",
            missing.tokens_in, missing.tokens_out
        );
    }

    #[test]
    fn legacy_audit_records_with_numeric_or_missing_usage_deserialize() {
        let base = serde_json::json!({
            "ordinal": 1,
            "agent_key": "agent",
            "display_name": "Agent",
            "phase": 0,
            "requested_model": "model",
            "critical": false,
            "quality_threshold": 0.5,
            "tool_access_level": "ReadOnly",
            "prompt_record_path": "prompt.json",
            "prompt_hash": "prompt",
            "system_hash": "system",
            "tools_hash": "tools",
            "output_path": "output.md",
            "output_hash": "hash",
            "cost_usd": 0.0,
            "duration_ms": 1,
            "quality": null,
            "tool_use_log": [],
            "attempts": [],
            "completed_at": "2026-10-14T00:00:00Z"
        });
        let mut numeric = base.clone();
        numeric["tokens_in"] = serde_json::json!(12);
        numeric["tokens_out"] = serde_json::json!(3);
        assert_legacy_audit_fields(numeric);

        let missing: AgentAuditRecord = serde_json::from_value(base).expect("missing usage");
        assert_eq!(missing.tokens_in, None);
        assert_eq!(missing.tokens_out, None);
        println!(
            "EVIDENCE #369 audit-record compatibility BEFORE: legacy numeric and absent token fields; AFTER: numeric audit deserialized and absent=({:?}, {:?})",
            missing.tokens_in, missing.tokens_out
        );
    }

    #[test]
    fn legacy_completed_attempt_events_with_numeric_or_missing_usage_deserialize() {
        let numeric: PipelineEvent = serde_json::from_value(serde_json::json!({
            "type": "llm_attempt_completed",
            "ordinal": 1,
            "agent_key": "agent",
            "attempt": 1,
            "output_hash": "hash",
            "tokens_in": 12,
            "tokens_out": 3,
            "duration_ms": 1
        }))
        .expect("numeric event usage");
        match numeric {
            PipelineEvent::LlmAttemptCompleted {
                tokens_in,
                tokens_out,
                ..
            } => {
                assert_eq!(tokens_in, Some(12));
                assert_eq!(tokens_out, Some(3));
            }
            _ => panic!("expected completed attempt"),
        }

        let missing: PipelineEvent = serde_json::from_value(serde_json::json!({
            "type": "llm_attempt_completed",
            "ordinal": 1,
            "agent_key": "agent",
            "attempt": 1,
            "output_hash": "hash",
            "duration_ms": 1
        }))
        .expect("missing event usage");
        match missing {
            PipelineEvent::LlmAttemptCompleted {
                tokens_in,
                tokens_out,
                ..
            } => {
                assert_eq!(tokens_in, None);
                assert_eq!(tokens_out, None);
            }
            _ => panic!("expected completed attempt"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineEventLine {
    pub ts: DateTime<Utc>,
    #[serde(flatten)]
    pub event: PipelineEvent,
}
