//! Normalizers for transcript and agent-output artifacts.

use serde_json::Value;

use crate::ingest::{IngestSummary, IngestWarning};
use crate::labels::DeterministicLabelBuilder;
use crate::schema::{EvidenceRef, WorldActionKind, WorldTraceRow, WorldTraceSource};

pub fn normalize_transcript_json(session_id: &str, json: &str) -> IngestSummary {
    let mut summary = IngestSummary {
        source: "transcript_json".into(),
        ..IngestSummary::default()
    };
    let value = match parse_json(json, &mut summary) {
        Some(value) => value,
        None => return summary,
    };

    for (idx, item) in transcript_items(&value).into_iter().enumerate() {
        let role = item
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let mut row = WorldTraceRow::new(session_id, WorldActionKind::Unknown)
            .with_row_id(format!("world-row-transcript-{session_id}-{idx}"))
            .with_evidence(EvidenceRef::new("agent_transcript", idx.to_string()));
        row.source = WorldTraceSource::AgentTranscript;
        row.agent = item
            .get("agent")
            .or_else(|| item.get("agent_key"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        row.redacted_excerpt = Some(format!("{role}: {}", text_from_value(item)));
        row.labels = DeterministicLabelBuilder.label_row(&row);
        summary.rows.push(row);
    }

    summary
}

pub fn normalize_transcript_jsonl(session_id: &str, jsonl: &str) -> IngestSummary {
    let mut summary = IngestSummary {
        source: "transcript_jsonl".into(),
        ..IngestSummary::default()
    };

    for (idx, line) in jsonl.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value = match serde_json::from_str::<Value>(trimmed) {
            Ok(value) => value,
            Err(error) => {
                summary.warnings.push(IngestWarning {
                    line: Some(line_no),
                    message: format!("invalid transcript jsonl: {error}"),
                });
                continue;
            }
        };
        let role = value
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let mut row = WorldTraceRow::new(session_id, WorldActionKind::Unknown)
            .with_row_id(format!("world-row-transcript-{session_id}-{idx}"))
            .with_evidence(EvidenceRef::new("agent_transcript", idx.to_string()));
        row.source = WorldTraceSource::AgentTranscript;
        row.agent = value
            .get("agent")
            .or_else(|| value.get("agent_key"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        row.redacted_excerpt = Some(format!("{role}: {}", text_from_value(&value)));
        row.labels = DeterministicLabelBuilder.label_row(&row);
        summary.rows.push(row);
    }

    summary
}

pub fn normalize_agent_output_json(session_id: &str, json: &str) -> IngestSummary {
    let mut summary = IngestSummary {
        source: "agent_output_json".into(),
        ..IngestSummary::default()
    };
    let value = match parse_json(json, &mut summary) {
        Some(value) => value,
        None => return summary,
    };

    for (idx, item) in output_items(&value).into_iter().enumerate() {
        let artifact_id = item
            .get("artifact_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("output");
        let mut row = WorldTraceRow::new(session_id, WorldActionKind::AgentAttempt)
            .with_row_id(format!(
                "world-row-agent-output-{session_id}-{artifact_id}-{idx}"
            ))
            .with_evidence(EvidenceRef::new("agent_output", artifact_id));
        row.source = WorldTraceSource::AgentOutput;
        row.agent = item
            .get("agent")
            .or_else(|| item.get("agent_key"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        row.run_id = item
            .get("run_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        row.redacted_excerpt = Some(text_from_value(item));
        row.labels = DeterministicLabelBuilder.label_row(&row);
        summary.rows.push(row);
    }

    summary
}

pub fn normalize_memory_json(session_id: &str, json: &str) -> IngestSummary {
    normalize_named_artifact_json(
        session_id,
        json,
        "memory_json",
        "memory",
        WorldTraceSource::Memory,
        WorldActionKind::MemorySurface,
    )
}

pub fn normalize_retrospective_json(session_id: &str, json: &str) -> IngestSummary {
    normalize_named_artifact_json(
        session_id,
        json,
        "retrospective_json",
        "retrospective",
        WorldTraceSource::Retrospective,
        WorldActionKind::Verification,
    )
}

pub fn normalize_agent_evolution_json(session_id: &str, json: &str) -> IngestSummary {
    normalize_named_artifact_json(
        session_id,
        json,
        "agent_evolution_json",
        "agent_evolution",
        WorldTraceSource::AgentEvolution,
        WorldActionKind::AgentAttempt,
    )
}

fn normalize_named_artifact_json(
    session_id: &str,
    json: &str,
    source: &str,
    evidence_kind: &str,
    trace_source: WorldTraceSource,
    action_kind: WorldActionKind,
) -> IngestSummary {
    let mut summary = IngestSummary {
        source: source.into(),
        ..IngestSummary::default()
    };
    let value = match parse_json(json, &mut summary) {
        Some(value) => value,
        None => return summary,
    };
    for (idx, item) in output_items(&value).into_iter().enumerate() {
        let item_id = item
            .get("id")
            .or_else(|| item.get("candidate_id"))
            .or_else(|| item.get("memory_id"))
            .and_then(Value::as_str)
            .unwrap_or(source);
        let mut row = WorldTraceRow::new(session_id, action_kind.clone())
            .with_row_id(format!("world-row-{source}-{session_id}-{idx}"))
            .with_evidence(EvidenceRef::new(evidence_kind, item_id));
        row.source = trace_source.clone();
        row.agent = item
            .get("agent")
            .or_else(|| item.get("agent_type"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        row.redacted_excerpt = Some(text_from_value(item));
        row.labels = DeterministicLabelBuilder.label_row(&row);
        summary.rows.push(row);
    }
    summary
}

fn parse_json(json: &str, summary: &mut IngestSummary) -> Option<Value> {
    match serde_json::from_str(json) {
        Ok(value) => Some(value),
        Err(error) => {
            summary.warnings.push(IngestWarning {
                line: None,
                message: format!("invalid artifact json: {error}"),
            });
            None
        }
    }
}

fn transcript_items(value: &Value) -> Vec<&Value> {
    array_field(value, "messages")
        .or_else(|| array_field(value, "turns"))
        .or_else(|| value.as_array())
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn output_items(value: &Value) -> Vec<&Value> {
    array_field(value, "outputs")
        .or_else(|| array_field(value, "artifacts"))
        .or_else(|| array_field(value, "memories"))
        .or_else(|| array_field(value, "candidates"))
        .or_else(|| array_field(value, "events"))
        .or_else(|| value.as_array())
        .map(|items| items.iter().collect())
        .unwrap_or_else(|| vec![value])
}

fn array_field<'a>(value: &'a Value, field: &str) -> Option<&'a Vec<Value>> {
    value.get(field).and_then(Value::as_array)
}

fn text_from_value(value: &Value) -> String {
    for key in ["content", "output", "text", "message"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return text.to_string();
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use crate::schema::{WorldActionKind, WorldTraceSource};

    use super::*;

    #[test]
    fn transcript_messages_become_agent_transcript_rows() {
        let json =
            r#"{"messages":[{"role":"assistant","agent_key":"coder","content":"verify tests"}]}"#;

        let summary = normalize_transcript_json("s1", json);

        assert_eq!(summary.row_count(), 1);
        assert_eq!(summary.rows[0].source, WorldTraceSource::AgentTranscript);
        assert_eq!(summary.rows[0].agent.as_deref(), Some("coder"));
        assert!(summary.rows[0].labels.verification_needed);
    }

    #[test]
    fn transcript_jsonl_lines_become_agent_transcript_rows() {
        let jsonl = r#"{"role":"assistant","agent_key":"coder","content":"completed"}"#;

        let summary = normalize_transcript_jsonl("s1", jsonl);

        assert_eq!(summary.row_count(), 1);
        assert_eq!(summary.rows[0].source, WorldTraceSource::AgentTranscript);
        assert_eq!(summary.rows[0].labels.success, Some(true));
    }

    #[test]
    fn output_objects_become_agent_output_rows() {
        let json =
            r#"{"artifact_id":"a1","agent":"implementer","output":"completed successfully"}"#;

        let summary = normalize_agent_output_json("s1", json);

        assert_eq!(summary.row_count(), 1);
        assert_eq!(summary.rows[0].source, WorldTraceSource::AgentOutput);
        assert_eq!(summary.rows[0].agent.as_deref(), Some("implementer"));
        assert_eq!(summary.rows[0].labels.success, Some(true));
    }

    #[test]
    fn memory_json_becomes_memory_surface_rows() {
        let summary = normalize_memory_json(
            "s1",
            r#"{"memories":[{"id":"m1","text":"remember verify"}]}"#,
        );

        assert_eq!(summary.source, "memory_json");
        assert_eq!(summary.row_count(), 1);
        assert_eq!(summary.rows[0].source, WorldTraceSource::Memory);
        assert_eq!(summary.rows[0].action_kind, WorldActionKind::MemorySurface);
    }
}
