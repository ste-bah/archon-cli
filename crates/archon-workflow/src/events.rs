use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::WorkflowResult;
use crate::store::WorkflowStore;

pub mod blocking_gap_events;
pub mod write_coordination_events;

const FORBIDDEN_FIELDS: &[&str] = &[
    "thinking",
    "reasoning",
    "reasoning_encrypted",
    "encrypted_reasoning",
    "oauth_token",
    "access_token",
    "refresh_token",
    "api_key",
    "authorization",
    "raw_text",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEventKind {
    Started,
    StageStarted,
    StageCompleted,
    StageFailed,
    StageStalled,
    StageSkipped,
    ForcedAccepted,
    Resumed,
    Paused,
    Cancelled,
    Completed,
    LearningRecorded,
    /// One residual gap with `severity: "blocking"` was recorded against a
    /// call. Emitted once per distinct blocking gap, carrying the gap's id and
    /// description, so `events.jsonl` names the same blockers `v2/results/`
    /// does. See [`blocking_gap_events`].
    BlockingGapDetected,
    WriteCoordinationItemWritePlanCreated,
    WriteCoordinationWaveScheduled,
    WriteCoordinationItemWorkspaceCreated,
    WriteCoordinationUndeclaredWriteDetected,
    WriteCoordinationPatchCaptured,
    WriteCoordinationPatchApplied,
    WriteCoordinationPatchConflict,
    WriteCoordinationWaveVerificationResult,
    WriteCoordinationDirectCanonicalMutationDetected,
    WriteCoordinationSerialFallback,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowEvent {
    pub seq: u64,
    pub run_id: String,
    pub ts: DateTime<Utc>,
    pub kind: WorkflowEventKind,
    pub detail: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactProgress {
    pub run_name: String,
    pub stage_index: usize,
    pub stage_total: usize,
    pub stage_id: String,
    pub active_agents: usize,
    pub completed_items: usize,
    pub total_items: usize,
    pub artifact_path: Option<String>,
}

impl CompactProgress {
    pub fn render(&self) -> String {
        if let Some(path) = &self.artifact_path {
            return format!("Workflow complete. Report: {path}");
        }
        format!(
            "Stage {}/{} {} running, {} agents active, {}/{} items complete",
            self.stage_index,
            self.stage_total,
            self.stage_id,
            self.active_agents,
            self.completed_items,
            self.total_items
        )
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowEventLog {
    store: WorkflowStore,
}

impl WorkflowEventLog {
    pub fn new(store: WorkflowStore) -> Self {
        Self { store }
    }

    pub fn emit(
        &self,
        run_id: &str,
        seq: u64,
        kind: WorkflowEventKind,
        detail: Value,
    ) -> WorkflowResult<WorkflowEvent> {
        let event = WorkflowEvent {
            seq,
            run_id: run_id.to_string(),
            ts: Utc::now(),
            kind,
            detail: sanitize_value(detail),
        };
        let line = serde_json::to_string(&event)?;
        self.store.append_event_line(run_id, &line)?;
        Ok(event)
    }
}

pub fn sanitize_value(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(sanitize_map(map)),
        Value::Array(items) => Value::Array(items.into_iter().map(sanitize_value).collect()),
        Value::String(text) => Value::String(redact_secret_like_text(&text)),
        other => other,
    }
}

fn sanitize_map(map: Map<String, Value>) -> Map<String, Value> {
    let mut cleaned = Map::new();
    for (key, value) in map {
        let lower = key.to_ascii_lowercase();
        if FORBIDDEN_FIELDS.iter().any(|field| lower.contains(field)) {
            continue;
        }
        cleaned.insert(key, sanitize_value(value));
    }
    cleaned
}

pub fn contains_forbidden_field(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            let lower = key.to_ascii_lowercase();
            FORBIDDEN_FIELDS.iter().any(|field| lower.contains(field))
                || contains_forbidden_field(value)
        }),
        Value::Array(items) => items.iter().any(contains_forbidden_field),
        _ => false,
    }
}

fn redact_secret_like_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut token = String::new();
    for ch in input.chars() {
        if ch.is_whitespace() {
            push_redacted_token(&mut out, &token);
            token.clear();
            out.push(ch);
        } else {
            token.push(ch);
        }
    }
    push_redacted_token(&mut out, &token);
    out
}

fn push_redacted_token(out: &mut String, token: &str) {
    if token.is_empty() {
        return;
    }
    if looks_secret_like(token) {
        out.push_str("<redacted>");
    } else {
        out.push_str(token);
    }
}

fn looks_secret_like(part: &str) -> bool {
    let trimmed = part.trim_matches(|ch: char| {
        matches!(ch, '"' | '\'' | '`' | ',' | ';' | ')' | '(' | '[' | ']')
    });
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("authorization:")
        || lower.starts_with("api_key=")
        || lower.starts_with("apikey=")
        || lower.starts_with("token=")
        || lower.starts_with("access_token=")
        || lower.starts_with("refresh_token=")
        || lower.starts_with("password=")
        || lower.starts_with("secret=")
        || lower.starts_with("sk-ant-")
        || (lower.starts_with("sk-") && trimmed.len() >= 20)
}
