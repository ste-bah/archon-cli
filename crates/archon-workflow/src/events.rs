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
mod pause_cause_tests {
    use super::{WorkflowEventLog, pause_cause, sanitize_value};
    use crate::{WorkflowEventKind, WorkflowStore};
    use serde_json::json;

    #[test]
    fn pause_cause_is_mandatory_and_uses_recorded_evidence_when_available() {
        let detail = pause_cause(
            json!({"action":"pause","pause_id":"call-7","reason":"stalled"}),
            "run-1",
        );
        assert_eq!(detail["cause_kind"], "pause");
        assert_eq!(detail["cause_reason"], "stalled");
        assert_eq!(detail["call_id"], "call-7");

        let detail = pause_cause(json!("legacy detail"), "run-2");
        assert_eq!(detail["detail"], "legacy detail");
        assert_eq!(detail["cause_kind"], "workflow_pause");
        assert_eq!(detail["cause_reason"], "legacy detail");
        assert_eq!(detail["call_id"], "run-control:run-2");

        let detail = pause_cause(json!({"evidence":"stdout cap=256, observed=300"}), "run-3");
        assert_eq!(detail["cause_reason"], "stdout cap=256, observed=300");

        let detail = pause_cause(
            json!({"event":"script_pause", "pause_id":"pause-7", "evidence":{"summary":"host judge needs review", "kind":"gate"}}),
            "run-3",
        );
        assert_eq!(detail["cause_reason"], "host judge needs review");
        assert_eq!(detail["call_id"], "pause-7");

        let detail = pause_cause(
            json!({
                "cause_kind": null, "event": "host_command_pause",
                "cause_reason": "", "evidence": "stderr output cap exceeded",
                "call_id": null, "pause_id": "call-8"
            }),
            "run-4",
        );
        assert_eq!(detail["cause_kind"], "host_command_pause");
        assert_eq!(detail["cause_reason"], "stderr output cap exceeded");
        assert_eq!(detail["call_id"], "call-8");
    }

    #[test]
    fn emitted_script_pause_event_sanitizes_nested_evidence_before_stringifying_it() {
        let temp = tempfile::tempdir().unwrap();
        let store = WorkflowStore::project(temp.path());
        let run = store
            .create_run(crate::WorkflowSpec {
                schema: crate::spec::WORKFLOW_SCHEMA.into(),
                name: "pause event".into(),
                task: "test".into(),
                target_repository_root: None,
                max_parallelism: 1,
                max_agents: 1,
                stages: Vec::new(),
                permissions: Default::default(),
                learning_hooks: Vec::new(),
            })
            .unwrap();
        let event = WorkflowEventLog::new(store)
            .emit(
                &run.id,
                1,
                WorkflowEventKind::Paused,
                json!({
                    "event":"script_pause",
                    "pause_id":"pause-acceptance-2",
                    "evidence":{
                        "kind":"host_command",
                        "context":{
                            "api_key":"api-secret",
                            "nested":{
                                "token":"token-secret",
                                "authorization":"Bearer auth-secret",
                                "password":"password-secret"
                            }
                        }
                    }
                }),
            )
            .unwrap();
        assert_eq!(event.detail["cause_kind"], "script_pause");
        let reason = event.detail["cause_reason"].as_str().unwrap();
        assert!(reason.contains("\"kind\":\"host_command\""), "{reason}");
        for secret in [
            "api-secret",
            "token-secret",
            "auth-secret",
            "password-secret",
        ] {
            assert!(!reason.contains(secret), "leaked {secret}: {reason}");
        }
        assert_eq!(event.detail["call_id"], "pause-acceptance-2");
    }

    #[test]
    fn nested_secret_fields_are_removed_before_evidence_becomes_cause_reason() {
        let evidence = json!({
            "category": "host_command",
            "context": {
                "api_key": "api-secret",
                "nested": {
                    "token": "token-secret",
                    "authorization": "Bearer auth-secret",
                    "password": "password-secret"
                }
            }
        });
        let sanitized = sanitize_value(json!({"evidence": evidence}));
        let detail = pause_cause(sanitized, "run-secret-test");
        let reason = detail["cause_reason"].as_str().unwrap();
        assert!(reason.contains("\"category\":\"host_command\""), "{reason}");
        for secret in [
            "api-secret",
            "token-secret",
            "auth-secret",
            "password-secret",
        ] {
            assert!(!reason.contains(secret), "leaked {secret}: {reason}");
        }
        assert!(!detail["evidence"].to_string().contains("api_key"));
        assert!(!detail["evidence"].to_string().contains("token\""));
        assert!(!detail["evidence"].to_string().contains("authorization"));
        assert!(!detail["evidence"].to_string().contains("password"));
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
