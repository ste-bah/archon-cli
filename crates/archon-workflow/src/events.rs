use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    "token",
    "api_key",
    "authorization",
    "password",
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
    /// A damaged acceptance round record was moved aside with its evidence
    /// and the progress ledger rebuilt without it (Issue 262). Carries the
    /// record, where its bytes went, why, and whether its state was known.
    AcceptanceRecordQuarantined,
    /// The acceptance record the final gate is bound to was damaged or
    /// gone, and the gate was rebuilt from the acceptance call's own result
    /// (Issue 262, round 9). Carries the record, why, and the call.
    AcceptanceGateRebuilt,
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
        // Sanitize structured input before deriving any string cause fields:
        // serializing evidence first would hide nested secret-shaped keys from
        // the recursive field redactor.
        let detail = sanitize_value(detail);
        let detail = if kind == WorkflowEventKind::Paused {
            pause_cause(detail, run_id)
        } else {
            detail
        };
        let event = WorkflowEvent {
            seq,
            run_id: run_id.to_string(),
            ts: Utc::now(),
            kind,
            detail,
        };
        let line = serde_json::to_string(&event)?;
        self.store.append_event_line(run_id, &line)?;
        Ok(event)
    }
}

fn pause_cause(detail: Value, run_id: &str) -> Value {
    let mut object = match detail {
        Value::Object(object) => object,
        other => serde_json::Map::from_iter([("detail".to_string(), other)]),
    };
    let kind = usable_cause_string(object.get("cause_kind"))
        .or_else(|| {
            usable_cause_string(object.get("event")).filter(|event| *event != "terminal_status")
        })
        .or_else(|| usable_cause_string(object.get("action")))
        .unwrap_or("workflow_pause")
        .to_string();
    let reason = usable_cause_string(object.get("cause_reason"))
        .or_else(|| usable_cause_string(object.get("cause")))
        .or_else(|| usable_cause_string(object.get("reason")))
        .or_else(|| usable_cause_string(object.get("error")))
        .or_else(|| usable_cause_string(object.get("detail")))
        .map(str::to_string)
        .or_else(|| structured_cause(object.get("evidence")))
        .unwrap_or_else(|| "workflow entered a resumable paused state".to_string());
    let call_id = usable_cause_string(object.get("call_id"))
        .or_else(|| usable_cause_string(object.get("pause_id")))
        .map(str::to_string)
        .unwrap_or_else(|| format!("run-control:{run_id}"));
    object.insert("cause_kind".into(), kind.into());
    object.insert("cause_reason".into(), reason.into());
    object.insert("call_id".into(), call_id.into());
    Value::Object(object)
}

/// Script pauses often carry evidence as a JSON object or array. Prefer its
/// human-written explanation fields, then retain the structured evidence in a
/// compact form so a pause event never loses its cause.
fn structured_cause(value: Option<&Value>) -> Option<String> {
    let value = value.filter(|value| !value.is_null())?;
    if let Some(text) = usable_cause_string(Some(value)) {
        return Some(text.to_string());
    }
    for field in ["summary", "reason", "message", "text", "detail"] {
        if let Some(text) = usable_cause_string(value.get(field)) {
            return Some(text.to_string());
        }
    }
    match value {
        Value::Object(_) | Value::Array(_) => serde_json::to_string(value).ok(),
        _ => None,
    }
}

fn usable_cause_string(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
#[path = "events_pause_cause_tests.rs"]
mod pause_cause_tests;

pub fn sanitize_value(value: Value) -> Value {
    fn remove_forbidden(value: Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.into_iter()
                    .filter(|(key, _)| {
                        let lower = key.to_ascii_lowercase();
                        !FORBIDDEN_FIELDS.iter().any(|field| lower.contains(field))
                    })
                    .map(|(key, value)| (key, remove_forbidden(value)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.into_iter().map(remove_forbidden).collect()),
            other => other,
        }
    }
    archon_observability::shared_redactor::redact_json(remove_forbidden(value), REDACTION_MARKER)
}

pub fn contains_forbidden_field(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, value)| {
            let lower = key.to_ascii_lowercase();
            archon_observability::shared_redactor::is_credential_key(key)
                || FORBIDDEN_FIELDS.iter().any(|field| lower.contains(field))
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

#[cfg(test)]
mod issue_444_redaction_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn review_corpus_matches_the_shared_redactor() {
        for (input, should_mask) in [
            ("Authorization: Bearer abc123", true),
            ("Proxy-Authorization: Basic abc123", true),
            ("api-key: abc123", true),
            ("Cookie: session=abc123", true),
            ("Set-Cookie: session=abc123", true),
            ("Bearer authentication", true),
            ("Bearer abc123", true),
            ("key=alpha", false),
            ("token=\"$3\"", false),
            ("curl -fsS -H token=\"x\" http://localhost/health", false),
            ("$HOME $PATH", false),
            ("token budget", false),
            ("the secret is nearby", false),
            ("password reset flow", false),
            ("Bearer tokens", true),
            ("https://alice:pass123@example.test", true),
            ("x-api-key: abc123", true),
            (r#"{"api_key":"abc123"}"#, true),
            (r#"{"apiKey":"abc123"}"#, true),
            (r#"{"access_token":"abc123"}"#, true),
            (r#"{"refresh_token":"abc123"}"#, true),
            (r#"{"client_secret":"abc123"}"#, true),
            (r#"{"password":"abc123"}"#, true),
            (r#"{"secret":"abc123"}"#, true),
            (r#"{"token":"abc123"}"#, true),
            (r#"{"auth_token":"abc123"}"#, true),
            (r#"{"private_key":"abc123"}"#, true),
            (r#"{"x-api-key":"abc123"}"#, true),
            (
                concat!(
                    "--",
                    "---BEGIN PRIVATE KEY-----\\nabc\\n-----END PRIVATE KEY-----"
                ),
                true,
            ),
            (concat!("sk", "-abcdefghijklmnopqrst0000"), true),
            (concat!("sk", "-ant-api03_ZZZZZZZZZZZZZZZZZZ1234"), true),
            (concat!("gh", "p_1234567890abcdef"), true),
            (concat!("gh", "o_1234567890abcdef"), true),
            (concat!("gh", "u_1234567890abcdef"), true),
            (concat!("gh", "s_1234567890abcdef"), true),
            (concat!("gh", "r_1234567890abcdef"), true),
            (concat!("gi", "thub_pat_1234567890abcdef"), true),
            (concat!("AK", "IA0123456789ABCDEF"), true),
            (concat!("AS", "IA0123456789ABCDEF"), true),
            (concat!("xo", "xb-1234567890-abcdef"), true),
            (concat!("sk", "_live_abcdefghijklmnopqrstuvwx"), true),
            (concat!("sk", "_test_abcdefghijklmnopqrstuvwx"), true),
            (concat!("pk", "_live_abcdefghijklmnopqrstuvwx"), true),
            (concat!("pk", "_test_abcdefghijklmnopqrstuvwx"), true),
            (concat!("rk", "_live_abcdefghijklmnopqrstuvwx"), true),
            (
                concat!("ey", "JhbGciOiJIUzI1NiI.eyJzdWIiOiIxMjM0NTY.SflKxwRJSMe"),
                true,
            ),
        ] {
            let output = sanitize_value(json!(input));
            assert_eq!(
                output.as_str().unwrap() == input,
                !should_mask,
                "{input:?} => {output:?}"
            );
        }
    }

    #[test]
    fn parsed_credential_fields_are_removed_without_removing_neighbor_values() {
        let output = sanitize_value(json!({"api_key":"abc123", "message":"token budget"}));
        assert!(output.get("api_key").is_none());
        assert_eq!(output["message"], "token budget");
    }
}
