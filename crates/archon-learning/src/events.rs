//! LearningEvent type and constructor helpers per LearningEventType.
//!
//! Each constructor creates a fully-formed LearningEvent with a generated ID
//! and timestamp. Persistence happens via `store::insert_learning_event`.

use cozo::DbInstance;

use crate::models::*;
use crate::store;

/// Build a new LearningEvent with generated ID and current timestamp.
pub fn new_event(
    workspace_id: &str,
    event_type: LearningEventType,
    source_artifact_id: &str,
    outcome_artifact_id: Option<&str>,
    signal: serde_json::Value,
    confidence: f32,
    provenance_record_id: &str,
) -> LearningEvent {
    let event_id = format!(
        "lev-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
    );
    LearningEvent {
        event_id,
        workspace_id: workspace_id.to_string(),
        event_type,
        source_artifact_id: source_artifact_id.to_string(),
        outcome_artifact_id: outcome_artifact_id.map(|s| s.to_string()),
        signal,
        confidence,
        provenance_record_id: provenance_record_id.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Create and persist a LearningEvent in one call.
pub fn record_event(
    db: &DbInstance,
    workspace_id: &str,
    event_type: LearningEventType,
    source_artifact_id: &str,
    outcome_artifact_id: Option<&str>,
    signal: serde_json::Value,
    confidence: f32,
    provenance_record_id: &str,
) -> Result<LearningEvent, crate::errors::LearningError> {
    let event = new_event(
        workspace_id,
        event_type,
        source_artifact_id,
        outcome_artifact_id,
        signal,
        confidence,
        provenance_record_id,
    );
    store::insert_learning_event(db, &event)
        .map_err(|e| crate::errors::LearningError::Storage { message: e.to_string() })?;
    Ok(event)
}

// ── Convenience constructors per event type ────────────────────────────────────

pub fn retrieval_used(
    workspace_id: &str,
    source_id: &str,
    chunk_id: Option<&str>,
    score: f32,
) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::RetrievalUsed,
        source_id,
        chunk_id,
        serde_json::json!({"score": score}),
        score,
        "",
    )
}

pub fn retrieval_rejected(
    workspace_id: &str,
    source_id: &str,
    chunk_id: Option<&str>,
    reason: &str,
) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::RetrievalRejected,
        source_id,
        chunk_id,
        serde_json::json!({"reason": reason}),
        0.8,
        "",
    )
}

pub fn source_confirmed(workspace_id: &str, source_id: &str, evidence: &str) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::SourceConfirmed,
        source_id,
        None,
        serde_json::json!({"evidence": evidence}),
        0.9,
        "",
    )
}

pub fn source_contradicted(
    workspace_id: &str,
    source_id: &str,
    contradiction_detail: &str,
) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::SourceContradicted,
        source_id,
        None,
        serde_json::json!({"contradiction": contradiction_detail}),
        0.8,
        "",
    )
}

pub fn gate_passed(workspace_id: &str, gate_name: &str, details: &str) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::GatePassed,
        gate_name,
        None,
        serde_json::json!({"details": details}),
        1.0,
        "",
    )
}

pub fn gate_failed(workspace_id: &str, gate_name: &str, reason: &str) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::GateFailed,
        gate_name,
        None,
        serde_json::json!({"reason": reason}),
        1.0,
        "",
    )
}

pub fn user_accepted(workspace_id: &str, artifact_id: &str) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::UserAccepted,
        artifact_id,
        None,
        serde_json::json!({}),
        1.0,
        "",
    )
}

pub fn user_corrected(
    workspace_id: &str,
    artifact_id: &str,
    correction: &str,
) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::UserCorrected,
        artifact_id,
        None,
        serde_json::json!({"correction": correction}),
        1.0,
        "",
    )
}

pub fn test_passed(workspace_id: &str, test_name: &str) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::TestPassed,
        test_name,
        None,
        serde_json::json!({}),
        1.0,
        "",
    )
}

pub fn test_failed(workspace_id: &str, test_name: &str, error: &str) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::TestFailed,
        test_name,
        None,
        serde_json::json!({"error": error}),
        1.0,
        "",
    )
}

pub fn false_completion_detected(
    workspace_id: &str,
    incident_id: &str,
    agent_key: Option<&str>,
    severity: &str,
) -> LearningEvent {
    new_event(
        workspace_id,
        LearningEventType::FalseCompletionDetected,
        incident_id,
        None,
        serde_json::json!({
            "incident_id": incident_id,
            "agent_key": agent_key,
            "severity": severity,
        }),
        0.9,
        "",
    )
}
