//! False-completion incident recorder.
//!
//! When a verified=false claim is contradicted by evidence or corrected by a user,
//! creates a [`FalseCompletionIncident`] and a linked governed-learning event.

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use std::collections::BTreeMap;

use crate::errors::EvidenceEngineError;
use crate::models::*;
use crate::store;

/// Record a false completion incident and a linked learning event.
pub fn record_false_completion(
    db: &DbInstance,
    claim: &CompletionClaim,
    actual_state: CompletionState,
    missing_evidence: Vec<EvidenceKind>,
    user_correction: Option<&str>,
) -> Result<FalseCompletionIncident, EvidenceEngineError> {
    let now = chrono::Utc::now().to_rfc3339();
    let incident_id = format!(
        "inc-{}",
        &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
    );
    let learning_event_id = format!(
        "le-{}",
        &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
    );

    // Compute severity based on claim kind and evidence gap
    let severity = compute_severity(
        &claim.claim_kind,
        &missing_evidence,
        user_correction.is_some(),
    );

    let claimed_state = format!("{:?}", claim.claim_kind);

    let incident = FalseCompletionIncident {
        incident_id,
        run_id: claim.run_id.clone(),
        agent_key: claim.agent_key.clone(),
        model: claim.model.clone(),
        task_type: claim.task_type.clone(),
        claimed_state,
        actual_state: actual_state.clone(),
        missing_evidence,
        contradiction_ids: claim.contradiction_ids.clone(),
        user_correction: user_correction.map(|s| s.to_string()),
        severity,
        learning_event_id: learning_event_id.clone(),
        created_at: now,
    };

    store::insert_false_completion_incident(db, &incident).map_err(|e| {
        EvidenceEngineError::Storage {
            message: e.to_string(),
        }
    })?;

    insert_canonical_learning_event(db, &incident).map_err(|e| EvidenceEngineError::Storage {
        message: e.to_string(),
    })?;

    Ok(incident)
}

fn compute_severity(
    kind: &CompletionClaimKind,
    missing_evidence: &[EvidenceKind],
    user_corrected: bool,
) -> IncidentSeverity {
    match kind {
        CompletionClaimKind::TestsPass | CompletionClaimKind::BuildPasses => {
            if user_corrected {
                IncidentSeverity::High
            } else if !missing_evidence.is_empty() {
                IncidentSeverity::Medium
            } else {
                IncidentSeverity::Low
            }
        }
        CompletionClaimKind::Done
        | CompletionClaimKind::Implemented
        | CompletionClaimKind::Fixed => {
            if user_corrected {
                IncidentSeverity::Critical
            } else {
                IncidentSeverity::High
            }
        }
        CompletionClaimKind::Ingested | CompletionClaimKind::Indexed => {
            if !missing_evidence.is_empty() {
                IncidentSeverity::Medium
            } else {
                IncidentSeverity::Low
            }
        }
        _ => IncidentSeverity::Low,
    }
}

fn insert_canonical_learning_event(
    db: &DbInstance,
    incident: &FalseCompletionIncident,
) -> Result<()> {
    archon_learning::schema::ensure_learning_schema(db)?;
    let workspace_id = workspace_id_for_run(db, &incident.run_id)?;
    let event = archon_learning::models::LearningEvent {
        event_id: incident.learning_event_id.clone(),
        workspace_id,
        event_type: archon_learning::models::LearningEventType::FalseCompletionDetected,
        source_artifact_id: incident.incident_id.clone(),
        outcome_artifact_id: Some(incident.run_id.clone()),
        signal: serde_json::json!({
            "incident_id": incident.incident_id,
            "run_id": incident.run_id,
            "agent_key": incident.agent_key,
            "model": incident.model,
            "task_type": incident.task_type,
            "claim_kind": incident.claimed_state,
            "actual_state": format!("{:?}", incident.actual_state),
            "severity": format!("{:?}", incident.severity),
            "missing_evidence": incident.missing_evidence,
            "contradiction_ids": incident.contradiction_ids,
            "user_correction": incident.user_correction,
        }),
        confidence: 0.9,
        provenance_record_id: String::new(),
        created_at: incident.created_at.clone(),
    };
    archon_learning::store::insert_learning_event(db, &event)?;
    Ok(())
}

fn workspace_id_for_run(db: &DbInstance, run_id: &str) -> Result<String> {
    let mut params = BTreeMap::new();
    params.insert("rid".into(), DataValue::from(run_id));
    let result = db.run_script(
        "?[workspace_id] := *completion_run_contexts{run_id, workspace_id}, run_id = $rid",
        params,
        ScriptMutability::Immutable,
    );

    match result {
        Ok(rows) => Ok(rows
            .rows
            .first()
            .and_then(|row| row.first())
            .and_then(|value| value.get_str())
            .filter(|workspace| !workspace.is_empty())
            .unwrap_or(crate::trust::DEFAULT_WORKSPACE_ID)
            .to_string()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Cannot find requested stored relation") {
                Ok(crate::trust::DEFAULT_WORKSPACE_ID.to_string())
            } else {
                Err(anyhow::anyhow!("read completion run context failed: {msg}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-incident-recorder-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_completion_schema(&db).unwrap();
        archon_learning::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    fn insert_run_context(db: &DbInstance, run_id: &str, workspace_id: &str) {
        store::insert_completion_run_context(
            db,
            &CompletionRunContext {
                run_id: run_id.to_string(),
                workspace_id: workspace_id.to_string(),
                agent_key: Some("ctx-agent".into()),
                model: Some("ctx-model".into()),
                updated_at: "2026-01-01T00:00:00Z".into(),
            },
        )
        .unwrap();
    }

    #[test]
    fn test_creates_false_completion_on_correction() {
        let db = test_db();
        let claim = CompletionClaim {
            claim_id: "cl-1".into(),
            run_id: "run-1".into(),
            agent_key: Some("test-agent".into()),
            model: Some("test-model".into()),
            task_type: "test".into(),
            claim_text: "All tests pass".into(),
            claim_kind: CompletionClaimKind::TestsPass,
            required_evidence: vec![EvidenceKind::TestRun],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        };

        let incident = record_false_completion(
            &db,
            &claim,
            CompletionState::Failed,
            vec![EvidenceKind::TestRun],
            Some("Tests actually failed"),
        )
        .unwrap();

        assert_eq!(incident.actual_state, CompletionState::Failed);
        assert_eq!(incident.severity, IncidentSeverity::High);
        assert!(!incident.learning_event_id.is_empty());

        // Verify persisted
        let all_incidents = store::get_all_incidents(&db).unwrap();
        assert_eq!(all_incidents.len(), 1);
        assert_eq!(all_incidents[0].incident_id, incident.incident_id);
    }

    #[test]
    fn test_links_learning_event() {
        let db = test_db();
        insert_run_context(&db, "run-2", "workspace-alpha");
        let claim = CompletionClaim {
            claim_id: "cl-2".into(),
            run_id: "run-2".into(),
            agent_key: None,
            model: None,
            task_type: "coding".into(),
            claim_text: "Done".into(),
            claim_kind: CompletionClaimKind::Done,
            required_evidence: vec![],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        };

        let incident = record_false_completion(
            &db,
            &claim,
            CompletionState::NotRun,
            vec![],
            Some("Not actually done"),
        )
        .unwrap();

        assert_eq!(incident.severity, IncidentSeverity::Critical);

        let event = archon_learning::store::get_learning_event(&db, &incident.learning_event_id)
            .unwrap()
            .expect("canonical learning event must exist");
        assert_eq!(event.event_id, incident.learning_event_id);
        assert_eq!(event.workspace_id, "workspace-alpha");
        assert_eq!(
            event.event_type,
            archon_learning::models::LearningEventType::FalseCompletionDetected
        );
        assert_eq!(event.source_artifact_id, incident.incident_id);
        assert_eq!(event.outcome_artifact_id.as_deref(), Some("run-2"));
        assert_eq!(event.signal["severity"], "Critical");
    }

    #[test]
    fn test_learning_event_id_set_for_phase6_consumption() {
        let db = test_db();
        let claim = CompletionClaim {
            claim_id: "cl-3".into(),
            run_id: "run-3".into(),
            agent_key: Some("test-agent".into()),
            model: None,
            task_type: "coding".into(),
            claim_text: "All done".into(),
            claim_kind: CompletionClaimKind::Done,
            required_evidence: vec![],
            linked_evidence_ids: vec![],
            verified: false,
            contradiction_ids: vec![],
            created_at: "2026-01-01T00:00:00Z".into(),
        };

        let incident = record_false_completion(
            &db,
            &claim,
            CompletionState::NotRun,
            vec![EvidenceKind::TestRun],
            Some("Incomplete work"),
        )
        .unwrap();

        // Cross-crate consumers depend on learning_event_id being set.
        assert!(
            !incident.learning_event_id.is_empty(),
            "learning_event_id must be set for downstream consumption"
        );
        assert!(
            incident.learning_event_id.starts_with("le-"),
            "learning_event_id must use the stable le- prefix"
        );
    }
}
