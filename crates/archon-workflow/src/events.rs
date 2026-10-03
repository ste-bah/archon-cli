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
    ScriptPreflightRejected,
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
    DecompositionPhaseStarted,
    AuthorAttemptStarted,
    /// The provider request for an author attempt is in flight. Distinct from
    /// `AuthorAttemptStarted`, which marks the logical attempt beginning.
    ModelCallInFlight,
    AuthorAttemptCompleted,
    AuthorAttemptInterrupted,
    AuthorAttemptRejected,
    HostCommandStarted,
    HostCommandCompleted,
    ShadowFindingsObserved,
    SubjectAccepted,
    SubjectAcceptedWithShadowFindings,
    DecompositionPhaseCompleted,
    DecompositionCompleted,
    /// A fixed decomposition resumed on a different build than the one that
    /// launched it (Issue-59). The persisted identity keeps the launch
    /// revision; this event, carrying `persisted` and `current`, is the record
    /// of the drift.
    BinaryRevisionDrift,
    /// A resume found the run `Running` while no live process held its
    /// executor lease: the previous owner died without a pause (Issue 251).
    /// Carries the lease evidence; the run moved to `Paused` with it.
    StaleOwnerRecovered,
    RunEndAcceptanceObserverStarted,
    RunEndAcceptanceShadowObserved,
    RunEndAcceptanceObserverFailed,
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
    /// A kind this build does not know, written by a newer one. Reading
    /// it as `Unknown` keeps an older build able to read the log at all:
    /// every reader of `events.jsonl` parses whole lines, so one unknown
    /// kind used to fail a resume, a finalizer or a status read (Issue 251).
    #[serde(other)]
    Unknown,
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

/// What [`sanitize_value`] writes in place of a secret-shaped word.
///
/// Redaction is for public copies only: events, prompt and agent-output
/// records, bundles and web views. Authoritative run state (v2 results,
/// branch outcomes, candidate artifacts, host-command stdin) is stored as
/// authored, so this marker appearing as a whole word in such data means a
/// redacted copy was read back as data; see [`redaction_marker_path`].
pub const REDACTION_MARKER: &str = "<redacted>";

fn redact_secret_like_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut token = String::new();
    let mut after_bearer = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            after_bearer = push_redacted_token(&mut out, &token, after_bearer);
            token.clear();
            out.push(ch);
        } else {
            token.push(ch);
        }
    }
    push_redacted_token(&mut out, &token, after_bearer);
    out
}

/// Writes one word, redacted when it is secret-shaped or is the credential
/// after a `Bearer` scheme word. Returns whether the NEXT word is such a
/// credential.
fn push_redacted_token(out: &mut String, token: &str, after_bearer: bool) -> bool {
    if token.is_empty() {
        return after_bearer;
    }
    if after_bearer || looks_secret_like(token) {
        out.push_str(REDACTION_MARKER);
    } else {
        out.push_str(token);
    }
    token
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`'))
        .eq_ignore_ascii_case("bearer")
}

/// The JSON pointer of the first string in `value` that holds
/// [`REDACTION_MARKER`] as a whole whitespace-delimited word, which is the
/// exact shape [`sanitize_value`] leaves behind.
///
/// A data-path reader about to freeze or stage an artifact calls this and
/// refuses the artifact naming the field, so a redacted display copy can never
/// silently become authoritative data. The marker inside other text (quoted,
/// or as part of a longer word) is not that shape and is not reported.
pub fn redaction_marker_path(value: &Value) -> Option<String> {
    fn walk(value: &Value, path: &mut String) -> bool {
        match value {
            Value::String(text) => text.split_whitespace().any(|word| word == REDACTION_MARKER),
            Value::Array(items) => items.iter().enumerate().any(|(index, item)| {
                let len = path.len();
                path.push_str(&format!("/{index}"));
                let found = walk(item, path);
                if !found {
                    path.truncate(len);
                }
                found
            }),
            Value::Object(map) => map.iter().any(|(key, item)| {
                let len = path.len();
                path.push('/');
                path.push_str(&key.replace('~', "~0").replace('/', "~1"));
                let found = walk(item, path);
                if !found {
                    path.truncate(len);
                }
                found
            }),
            _ => false,
        }
    }
    let mut path = String::new();
    walk(value, &mut path).then_some(path)
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
