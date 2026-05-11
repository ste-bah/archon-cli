use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use crate::labels::DeterministicLabelBuilder;
use crate::schema::{
    EvidenceRef, ScalarFeatures, WorldActionKind, WorldTraceRow, WorldTraceSource,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestWarning {
    pub line: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct IngestSummary {
    pub source: String,
    pub rows: Vec<WorldTraceRow>,
    pub warnings: Vec<IngestWarning>,
}

impl IngestSummary {
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn warning_count(&self) -> usize {
        self.warnings.len()
    }
}

#[derive(Debug, Clone, Deserialize)]
struct RawActivityEvent {
    event_id: String,
    session_id: String,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    agent_key: Option<String>,
    #[serde(default)]
    subagent_type: Option<String>,
    kind: String,
    status: String,
    message: String,
    #[serde(default)]
    artifact_id: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    cost_usd: Option<f64>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawTraceExportRow {
    session_id: String,
    agent_key: String,
    ordinal: usize,
    phase: u32,
    action: String,
    attempt: usize,
    accepted: bool,
    failure_reason: Option<String>,
    prompt_hash: String,
    output_hash: String,
    tokens_in: u64,
    tokens_out: u64,
    quality_overall: Option<f64>,
    verifier_status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawProviderRuntimeEvent {
    event_id: String,
    provider_id: String,
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    model_id: Option<String>,
    #[serde(default)]
    event_type: String,
    #[serde(default)]
    severity: String,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    retry_count: Option<u32>,
    #[serde(default)]
    run_id: Option<String>,
    #[serde(default)]
    pipeline_id: Option<String>,
    created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPlanDocument {
    id: String,
    title: String,
    #[serde(default)]
    steps: Vec<RawPlanStep>,
    #[serde(default)]
    status: String,
}

#[derive(Debug, Clone, Deserialize)]
struct RawPlanStep {
    number: u32,
    description: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    affected_files: Vec<String>,
}

pub fn normalize_activity_jsonl(input: &str) -> IngestSummary {
    normalize_jsonl("activity_jsonl", input, normalize_activity_event)
}

pub fn normalize_trace_export_jsonl(input: &str) -> IngestSummary {
    normalize_jsonl("trace_export_jsonl", input, normalize_trace_export_row)
}

pub fn normalize_provider_runtime_jsonl(input: &str) -> IngestSummary {
    normalize_jsonl(
        "provider_runtime_jsonl",
        input,
        normalize_provider_runtime_event,
    )
}

pub fn normalize_conversation_messages(session_id: &str, messages: &[Value]) -> IngestSummary {
    let mut summary = IngestSummary {
        source: "conversation_messages".into(),
        ..IngestSummary::default()
    };

    for (idx, message) in messages.iter().enumerate() {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let content = message_text(message);
        let mut row = WorldTraceRow::new(session_id, WorldActionKind::Unknown)
            .with_row_id(format!("world-row-conversation-{session_id}-{idx}"))
            .with_evidence(EvidenceRef::new("conversation_message", idx.to_string()));
        row.source = WorldTraceSource::Conversation;
        row.redacted_excerpt = Some(format!("{role}: {content}"));
        row.labels = DeterministicLabelBuilder.label_row(&row);
        summary.rows.push(row);
    }

    summary
}

pub fn normalize_plan_json(session_id: &str, json: &str) -> IngestSummary {
    let mut summary = IngestSummary {
        source: "plan_json".into(),
        ..IngestSummary::default()
    };

    let plan: RawPlanDocument = match serde_json::from_str(json) {
        Ok(plan) => plan,
        Err(error) => {
            summary.warnings.push(IngestWarning {
                line: None,
                message: format!("invalid plan json: {error}"),
            });
            return summary;
        }
    };

    for step in plan.steps {
        let mut row = WorldTraceRow::new(session_id, WorldActionKind::PlanUpdate)
            .with_row_id(format!(
                "world-row-plan-{session_id}-{}-{}",
                plan.id, step.number
            ))
            .with_evidence(EvidenceRef::new("plan", plan.id.clone()));
        row.source = WorldTraceSource::Plan;
        row.run_id = Some(plan.id.clone());
        row.redacted_excerpt = Some(format!(
            "plan={} status={} step={} step_status={} description={} files={}",
            plan.title,
            plan.status,
            step.number,
            step.status,
            step.description,
            step.affected_files.join(",")
        ));
        row.labels.plan_drift = plan.status == "abandoned" || step.status == "skipped";
        row.labels = DeterministicLabelBuilder.label_row(&row);
        if row.labels.plan_drift {
            row.labels.plan_drift = true;
        }
        summary.rows.push(row);
    }

    summary
}

fn normalize_jsonl<T>(
    source: &str,
    input: &str,
    normalize: impl Fn(T) -> WorldTraceRow,
) -> IngestSummary
where
    T: for<'de> Deserialize<'de>,
{
    let mut summary = IngestSummary {
        source: source.into(),
        ..IngestSummary::default()
    };

    for (idx, line) in input.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<T>(trimmed) {
            Ok(raw) => summary.rows.push(normalize(raw)),
            Err(error) => summary.warnings.push(IngestWarning {
                line: Some(line_no),
                message: error.to_string(),
            }),
        }
    }

    summary
}

fn normalize_activity_event(event: RawActivityEvent) -> WorldTraceRow {
    let action_kind = activity_action_kind(&event.kind);
    let row_id = format!("world-row-activity-{}", event.event_id);
    let mut row = WorldTraceRow::new(event.session_id, action_kind)
        .with_row_id(row_id)
        .with_evidence(EvidenceRef::new("activity_event", event.event_id));
    if let Some(artifact_id) = event.artifact_id {
        row.evidence_refs
            .push(EvidenceRef::new("activity_artifact", artifact_id));
    }
    row.source = WorldTraceSource::ActivityEvent;
    row.run_id = event.run_id;
    row.provider = event.provider;
    row.model = event.model;
    row.agent = event.agent_key.or(event.subagent_type);
    row.scalar_features = ScalarFeatures {
        cost_usd: event.cost_usd,
        ..ScalarFeatures::default()
    };
    row.redacted_excerpt = Some(event.message);
    row.created_at = event.created_at;
    row.labels = DeterministicLabelBuilder.label_row(&row);

    let status = event.status.as_str();
    if status == "failed" || event.kind.ends_with("_failed") {
        row.labels.failure = true;
        row.labels.success = Some(false);
    } else if status == "completed" {
        row.labels.success = Some(true);
    }

    row
}

fn normalize_trace_export_row(trace: RawTraceExportRow) -> WorldTraceRow {
    let row_id = format!(
        "world-row-trace-{}-{}-{}",
        trace.session_id, trace.ordinal, trace.attempt
    );
    let mut row = WorldTraceRow::new(trace.session_id, WorldActionKind::AgentAttempt)
        .with_row_id(row_id)
        .with_evidence(EvidenceRef::new("trace_export", trace.output_hash.clone()));
    row.source = WorldTraceSource::PipelineBundle;
    row.run_id = Some(format!("phase-{}-ordinal-{}", trace.phase, trace.ordinal));
    row.agent = Some(trace.agent_key);
    row.scalar_features = ScalarFeatures {
        attempt_index: Some(trace.attempt as u32),
        tokens_in: Some(trace.tokens_in),
        tokens_out: Some(trace.tokens_out),
        quality_overall: trace.quality_overall,
        ..ScalarFeatures::default()
    };
    row.labels.success = Some(trace.accepted);
    row.labels.failure = !trace.accepted || trace.failure_reason.is_some();
    row.labels.retry = trace.attempt > 1;
    row.labels.verification_needed = trace.verifier_status != "verified";
    row.redacted_excerpt = Some(format!(
        "action={} accepted={} verifier={} prompt_hash={} failure={}",
        trace.action,
        trace.accepted,
        trace.verifier_status,
        trace.prompt_hash,
        trace.failure_reason.unwrap_or_default()
    ));
    row
}

fn normalize_provider_runtime_event(event: RawProviderRuntimeEvent) -> WorldTraceRow {
    let row_id = format!("world-row-provider-{}", event.event_id);
    let created_at = DateTime::parse_from_rfc3339(&event.created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let mut row = WorldTraceRow::new(
        event
            .pipeline_id
            .clone()
            .unwrap_or_else(|| event.provider_id.clone()),
        WorldActionKind::ProviderCall,
    )
    .with_row_id(row_id)
    .with_evidence(EvidenceRef::new("provider_runtime_event", event.event_id));
    row.source = WorldTraceSource::ProviderRuntime;
    row.run_id = event.run_id.or(event.pipeline_id);
    row.provider = Some(event.provider_id);
    row.model = event.model_id;
    row.redacted_excerpt = Some(format!(
        "profile={} event_type={} severity={} message={}",
        event.profile_id.unwrap_or_default(),
        event.event_type,
        event.severity,
        event.message.unwrap_or_default()
    ));
    row.created_at = created_at;
    row.labels = DeterministicLabelBuilder.label_row(&row);
    row.labels.retry = event.retry_count.unwrap_or(0) > 0;

    if event.event_type.contains("failed")
        || event.event_type.contains("denied")
        || event.severity == "error"
    {
        row.labels.failure = true;
        row.labels.success = Some(false);
        row.labels.provider_incident = true;
    }

    if event.event_type.contains("rate_limit")
        || event.event_type.contains("usage_limit")
        || event.event_type.contains("cooldown")
    {
        row.labels.provider_incident = true;
    }

    row
}

fn activity_action_kind(kind: &str) -> WorldActionKind {
    match kind {
        "tool_started" | "tool_completed" | "tool_failed" => WorldActionKind::ToolCall,
        "memory_surfaced" => WorldActionKind::MemorySurface,
        "agent_queued"
        | "agent_spawned"
        | "agent_running"
        | "agent_completed"
        | "agent_failed"
        | "pipeline_specialist_started"
        | "pipeline_specialist_completed" => WorldActionKind::AgentAttempt,
        "agent_waiting_provider" => WorldActionKind::ProviderCall,
        _ => WorldActionKind::Unknown,
    }
}

fn message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) => value.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_jsonl_is_tolerant_and_normalizes_rows() {
        let input = r#"{"event_id":"e1","session_id":"s1","kind":"tool_failed","status":"failed","message":"tool failed","created_at":"2026-05-10T12:00:00Z"}"#;
        let summary = normalize_activity_jsonl(&format!("{input}\nnot-json\n"));

        assert_eq!(summary.row_count(), 1);
        assert_eq!(summary.warning_count(), 1);
        let row = &summary.rows[0];
        assert_eq!(row.source, WorldTraceSource::ActivityEvent);
        assert_eq!(row.action_kind, WorldActionKind::ToolCall);
        assert!(row.labels.failure);
    }

    #[test]
    fn trace_export_rows_capture_attempt_features() {
        let input = r#"{"session_id":"s1","agent_key":"implementer","ordinal":2,"phase":1,"action":"agent_attempt","attempt":2,"accepted":false,"failure_reason":"tests failed","prompt_hash":"ph","output_hash":"oh","tokens_in":10,"tokens_out":20,"quality_overall":0.4,"verifier_status":"unverified"}"#;
        let summary = normalize_trace_export_jsonl(input);

        assert_eq!(summary.row_count(), 1);
        let row = &summary.rows[0];
        assert_eq!(row.source, WorldTraceSource::PipelineBundle);
        assert_eq!(row.agent.as_deref(), Some("implementer"));
        assert_eq!(row.scalar_features.attempt_index, Some(2));
        assert_eq!(row.scalar_features.tokens_in, Some(10));
        assert!(row.labels.retry);
        assert!(row.labels.verification_needed);
    }

    #[test]
    fn provider_runtime_rows_mark_incidents() {
        let input = r#"{"event_id":"p1","provider_id":"anthropic","model_id":"claude","runtime_mode":"oauth","event_type":"rate_limit_observed","severity":"warn","retry_count":1,"created_at":"2026-05-10T12:00:00Z"}"#;
        let summary = normalize_provider_runtime_jsonl(input);

        assert_eq!(summary.row_count(), 1);
        let row = &summary.rows[0];
        assert_eq!(row.source, WorldTraceSource::ProviderRuntime);
        assert_eq!(row.action_kind, WorldActionKind::ProviderCall);
        assert!(row.labels.provider_incident);
        assert!(row.labels.retry);
    }

    #[test]
    fn conversation_messages_become_rows() {
        let messages = vec![serde_json::json!({"role":"user","content":"please verify this"})];
        let summary = normalize_conversation_messages("s1", &messages);

        assert_eq!(summary.row_count(), 1);
        assert_eq!(summary.rows[0].source, WorldTraceSource::Conversation);
        assert!(summary.rows[0].labels.verification_needed);
    }

    #[test]
    fn plan_json_marks_skipped_as_drift() {
        let json = r#"{"id":"p1","title":"Plan","status":"active","steps":[{"number":1,"description":"Do it","status":"skipped","affected_files":["a.rs"]}]}"#;
        let summary = normalize_plan_json("s1", json);

        assert_eq!(summary.row_count(), 1);
        assert_eq!(summary.rows[0].source, WorldTraceSource::Plan);
        assert!(summary.rows[0].labels.plan_drift);
    }
}
