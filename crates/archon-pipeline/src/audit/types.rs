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
pub struct AgentAttemptRecord {
    pub attempt: usize,
    #[serde(default)]
    pub output_path: Option<String>,
    pub output_hash: String,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub duration_ms: u64,
    pub quality: Option<QualityScore>,
    pub accepted: bool,
    pub failure_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub tokens_in: u64,
    pub tokens_out: u64,
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
        tokens_in: u64,
        tokens_out: u64,
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineEventLine {
    pub ts: DateTime<Utc>,
    #[serde(flatten)]
    pub event: PipelineEvent,
}
