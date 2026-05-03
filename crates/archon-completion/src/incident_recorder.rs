//! False-completion incident recorder.
//!
//! When a verified=false claim is contradicted by evidence or corrected by a user,
//! creates a [`FalseCompletionIncident`] and a parallel learning event for Phase 6.

use anyhow::Result;
use cozo::DbInstance;

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
        uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
    );
    let learning_event_id = format!(
        "le-{}",
        uuid::Uuid::new_v4().to_string().replace('-', "")[..12].to_string()
    );

    // Compute severity based on claim kind and evidence gap
    let severity = compute_severity(&claim.claim_kind, &missing_evidence, user_correction.is_some());

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

    store::insert_false_completion_incident(db, &incident)
        .map_err(|e| EvidenceEngineError::Storage { message: e.to_string() })?;

    // Also insert a placeholder learning event for Phase 6 to consume.
    // Uses a raw Cozo script since the learning_events relation is in the gametheory schema.
    insert_learning_event_pending(db, &learning_event_id, &incident)
        .map_err(|e| EvidenceEngineError::Storage { message: e.to_string() })?;

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
        CompletionClaimKind::Done | CompletionClaimKind::Implemented | CompletionClaimKind::Fixed => {
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

/// Insert a placeholder learning event row for Phase 6 consumption.
fn insert_learning_event_pending(
    db: &DbInstance,
    event_id: &str,
    incident: &FalseCompletionIncident,
) -> Result<()> {
    use std::collections::BTreeMap;
    use cozo::{DataValue, ScriptMutability};

    // Ensure the learn_events relation exists (it may be in gametheory schema).
    // We try to insert; if the relation doesn't exist, this is non-fatal for Phase 5.
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(event_id));
    params.insert("wid".into(), DataValue::from("default"));
    params.insert("et".into(), DataValue::from("FalseCompletionDetected"));
    params.insert("sid".into(), DataValue::from(""));
    params.insert("oid".into(), DataValue::from(""));
    params.insert("sig".into(), DataValue::from(
        serde_json::json!({
            "incident_id": incident.incident_id,
            "claim_kind": incident.claimed_state,
            "actual_state": format!("{:?}", incident.actual_state),
            "severity": format!("{:?}", incident.severity),
        })
        .to_string()
        .as_str(),
    ));
    params.insert("cf".into(), DataValue::from(0.8_f64));
    params.insert("prid".into(), DataValue::from(""));
    params.insert("ca".into(), DataValue::from(incident.created_at.as_str()));

    let result = db.run_script(
        "?[event_id, workspace_id, event_type, source_artifact_id, \
         outcome_artifact_id, signal, confidence, provenance_record_id, created_at] \
         <- [[$eid, $wid, $et, $sid, $oid, $sig, $cf, $prid, $ca]] \
         :put learn_events { event_id => workspace_id, event_type, \
         source_artifact_id, outcome_artifact_id, signal, confidence, \
         provenance_record_id, created_at }",
        params,
        ScriptMutability::Mutable,
    );

    // Non-fatal: relation may not exist until gametheory schema is set up.
    if let Err(ref e) = result {
        let msg = e.to_string();
        if msg.contains("Cannot find requested stored relation") {
            tracing::warn!(
                event_id = %event_id,
                "learn_events relation not found — learning event deferred to Phase 6"
            );
            return Ok(());
        }
    }

    result.map(|_| ()).map_err(|e| anyhow::anyhow!("insert learning event failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-incident-recorder-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        // Ensure completion schema
        crate::schema::ensure_completion_schema(&db).unwrap();
        // Ensure learn_events exists (gametheory schema)
        let _ = db.run_script(
            ":create learn_events { event_id: String => workspace_id: String, event_type: String, \
             source_artifact_id: String default \"\", outcome_artifact_id: String default \"\", \
             signal: String default \"{}\", confidence: Float default 0.5, \
             provenance_record_id: String default \"\", created_at: String }",
            Default::default(),
            cozo::ScriptMutability::Mutable,
        );
        db
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

        // Verify learning event was persisted
        let events = db
            .run_script(
                "?[event_id] := *learn_events{event_id, event_type: \"FalseCompletionDetected\"}",
                Default::default(),
                cozo::ScriptMutability::Immutable,
            )
            .unwrap();
        assert_eq!(events.rows.len(), 1);
    }
}
