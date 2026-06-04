//! Outcome signal translators — convert incoming events from completion-integrity sources
//! into canonical LearningEvent records.
//!
//! Three signal sources:
//! 1. Completion incidents (legacy learn_events + false_completion_incidents)
//! 2. Gametheory gate/test outcomes
//! 3. User CLI accept/reject

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::errors::COZO_RELATION_NOT_FOUND;
use crate::models::*;
use crate::store;

/// Consume pending rows from the legacy `learn_events` bridge relation,
/// translate them into canonical LearningEvent rows in `learning_events`, then
/// delete the consumed rows.
///
/// Returns the count of events consumed.
pub fn consume_completion_events(db: &DbInstance) -> Result<usize> {
    crate::schema::ensure_learning_schema(db)?;

    // Read all rows from the legacy learn_events relation.
    let pending = match read_learn_events(db) {
        Ok(rows) => rows,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains(COZO_RELATION_NOT_FOUND) {
                return Ok(0); // No Phase 5 data yet, nothing to consume
            }
            return Err(e);
        }
    };

    if pending.is_empty() {
        return Ok(0);
    }

    let mut consumed = 0;
    for row in &pending {
        let event = translate_learn_event_row(row);
        store::insert_learning_event(db, &event)?;
        consumed += 1;
    }

    // Delete consumed rows from learn_events
    delete_learn_events(db)?;

    Ok(consumed)
}

/// Read raw rows from the legacy `learn_events` relation.
fn read_learn_events(db: &DbInstance) -> Result<Vec<Vec<DataValue>>> {
    let result = db
        .run_script(
            "?[event_id, workspace_id, event_type, source_artifact_id, \
             outcome_artifact_id, signal, confidence, provenance_record_id, created_at] \
             := *learn_events{event_id, workspace_id, event_type, \
             source_artifact_id, outcome_artifact_id, signal, confidence, \
             provenance_record_id, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("read learn_events failed: {e}"))?;
    Ok(result.rows)
}

/// Translate a legacy learn_events row into a LearningEvent.
fn translate_learn_event_row(row: &[DataValue]) -> LearningEvent {
    LearningEvent {
        event_id: format!(
            "lev-{}",
            &uuid::Uuid::new_v4().to_string().replace('-', "")[..12]
        ),
        workspace_id: row
            .get(1)
            .and_then(|v| v.get_str())
            .unwrap_or("default")
            .to_string(),
        event_type: row
            .get(2)
            .and_then(|v| v.get_str())
            .and_then(LearningEventType::from_str)
            .unwrap_or(LearningEventType::FalseCompletionDetected),
        source_artifact_id: row
            .get(3)
            .and_then(|v| v.get_str())
            .unwrap_or("")
            .to_string(),
        outcome_artifact_id: {
            let s = row.get(4).and_then(|v| v.get_str()).unwrap_or("");
            if s.is_empty() {
                None
            } else {
                Some(s.to_string())
            }
        },
        signal: row
            .get(5)
            .and_then(|v| v.get_str())
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(serde_json::Value::Object(Default::default())),
        confidence: row.get(6).and_then(|v| v.get_float()).unwrap_or(0.5) as f32,
        provenance_record_id: row
            .get(7)
            .and_then(|v| v.get_str())
            .unwrap_or("")
            .to_string(),
        created_at: row
            .get(8)
            .and_then(|v| v.get_str())
            .unwrap_or("")
            .to_string(),
    }
}

/// Delete all rows from the legacy `learn_events` relation after consumption.
fn delete_learn_events(db: &DbInstance) -> Result<()> {
    // Cozo doesn't support :delete with conditions directly in all versions;
    // we remove by extracting all keys and then removing them.
    let rows = read_learn_events(db)?;
    if rows.is_empty() {
        return Ok(());
    }

    // Use :rm to delete by key
    for row in &rows {
        if let Some(event_id) = row.first().and_then(|v| v.get_str()) {
            let mut params = BTreeMap::new();
            params.insert("eid".into(), DataValue::from(event_id));
            let _ = db.run_script(
                "?[event_id] <- [[$eid]] :rm learn_events { event_id }",
                params,
                ScriptMutability::Mutable,
            );
        }
    }
    Ok(())
}

/// Consume false_completion_incidents and emit FalseCompletionDetected learning events.
/// Only processes incidents that haven't already been translated (those without a
/// corresponding learning event in the canonical `learning_events` relation).
pub fn consume_incident_events(db: &DbInstance) -> Result<usize> {
    crate::schema::ensure_learning_schema(db)?;

    // Read false_completion_incidents that have a non-empty learning_event_id
    let result = db.run_script(
        "?[incident_id, run_id, agent_key, model, task_type, claimed_state, \
             actual_state, missing_evidence_json, user_correction, severity, \
             learning_event_id, created_at] \
             := *false_completion_incidents{incident_id, run_id, agent_key, model, \
             task_type, claimed_state, actual_state, missing_evidence_json, \
             user_correction, severity, learning_event_id, created_at}, \
             learning_event_id != \"\"",
        Default::default(),
        ScriptMutability::Immutable,
    );

    let rows = match result {
        Ok(r) => r.rows,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains(COZO_RELATION_NOT_FOUND) {
                return Ok(0);
            }
            return Err(anyhow::anyhow!("consume incidents failed: {msg}"));
        }
    };

    let mut consumed = 0;
    for row in &rows {
        let incident_id = row[0].get_str().unwrap_or("").to_string();
        let agent_key = row[2].get_str().map(|s| s.to_string());
        let severity = row[9].get_str().unwrap_or("Low").to_string();
        let learning_event_id = row[10].get_str().unwrap_or("").to_string();

        if learning_event_id.is_empty()
            || store::get_learning_event(db, &learning_event_id)?.is_some()
        {
            continue;
        }

        let mut event = crate::events::false_completion_detected(
            "default",
            &incident_id,
            agent_key.as_deref(),
            &severity,
        );
        event.event_id = learning_event_id;
        store::insert_learning_event(db, &event)?;
        consumed += 1;
    }

    Ok(consumed)
}

/// Record a user acceptance or correction as a LearningEvent.
pub fn record_user_signal(
    db: &DbInstance,
    workspace_id: &str,
    accepted: bool,
    artifact_id: &str,
    correction: Option<&str>,
) -> Result<LearningEvent> {
    let result = if accepted {
        crate::events::record_event(
            db,
            workspace_id,
            LearningEventType::UserAccepted,
            artifact_id,
            None,
            serde_json::json!({}),
            1.0,
            "",
        )
    } else {
        crate::events::record_event(
            db,
            workspace_id,
            LearningEventType::UserCorrected,
            artifact_id,
            None,
            serde_json::json!({"correction": correction.unwrap_or("")}),
            1.0,
            "",
        )
    };
    result.map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-outcome-signal-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        // Also create the legacy learn_events relation for consume tests.
        create_phase5_learn_events(&db);
        db
    }

    fn create_phase5_learn_events(db: &DbInstance) {
        let _ = db.run_script(
            ":create learn_events { event_id: String => workspace_id: String, event_type: String, \
             source_artifact_id: String default \"\", outcome_artifact_id: String default \"\", \
             signal: String default \"{}\", confidence: Float default 0.5, \
             provenance_record_id: String default \"\", created_at: String }",
            Default::default(),
            ScriptMutability::Mutable,
        );
    }

    fn create_false_completion_incidents(db: &DbInstance) {
        let _ = db.run_script(
            ":create false_completion_incidents {
                incident_id: String =>
                run_id: String,
                agent_key: String default \"\",
                model: String default \"\",
                task_type: String,
                claimed_state: String,
                actual_state: String,
                missing_evidence_json: String default \"[]\",
                user_correction: String default \"\",
                severity: String,
                learning_event_id: String default \"\",
                created_at: String,
            }",
            Default::default(),
            ScriptMutability::Mutable,
        );
    }

    fn insert_incident(db: &DbInstance, incident_id: &str, event_id: &str) {
        let mut params = BTreeMap::new();
        params.insert("iid".into(), DataValue::from(incident_id));
        params.insert("rid".into(), DataValue::from("run-incident-1"));
        params.insert("ak".into(), DataValue::from("agent-alpha"));
        params.insert("model".into(), DataValue::from("model-alpha"));
        params.insert("tt".into(), DataValue::from("coding"));
        params.insert("cs".into(), DataValue::from("Done"));
        params.insert("actual".into(), DataValue::from("NotRun"));
        params.insert("me".into(), DataValue::from("[\"TestRun\"]"));
        params.insert("uc".into(), DataValue::from(""));
        params.insert("sev".into(), DataValue::from("High"));
        params.insert("leid".into(), DataValue::from(event_id));
        params.insert("ca".into(), DataValue::from("2026-05-04T00:00:00Z"));
        db.run_script(
            "?[incident_id, run_id, agent_key, model, task_type, claimed_state, \
             actual_state, missing_evidence_json, user_correction, severity, \
             learning_event_id, created_at] \
             <- [[$iid, $rid, $ak, $model, $tt, $cs, $actual, $me, $uc, $sev, $leid, $ca]] \
             :put false_completion_incidents { incident_id => run_id, agent_key, model, \
             task_type, claimed_state, actual_state, missing_evidence_json, \
             user_correction, severity, learning_event_id, created_at }",
            params,
            ScriptMutability::Mutable,
        )
        .unwrap();
    }

    #[test]
    fn test_consume_pending_translates_completion_events() {
        let db = test_db();

        // Insert a row into legacy learn_events.
        let mut params = BTreeMap::new();
        params.insert("eid".into(), DataValue::from("le-phase5-001"));
        params.insert("wid".into(), DataValue::from("default"));
        params.insert("et".into(), DataValue::from("FalseCompletionDetected"));
        params.insert("sid".into(), DataValue::from("inc-001"));
        params.insert("oid".into(), DataValue::from(""));
        params.insert("sig".into(), DataValue::from("{\"severity\":\"High\"}"));
        params.insert("cf".into(), DataValue::from(0.8_f64));
        params.insert("prid".into(), DataValue::from(""));
        params.insert("ca".into(), DataValue::from("2026-05-03T00:00:00Z"));

        db.run_script(
            "?[event_id, workspace_id, event_type, source_artifact_id, \
             outcome_artifact_id, signal, confidence, provenance_record_id, created_at] \
             <- [[$eid, $wid, $et, $sid, $oid, $sig, $cf, $prid, $ca]] \
             :put learn_events { event_id => workspace_id, event_type, \
             source_artifact_id, outcome_artifact_id, signal, confidence, \
             provenance_record_id, created_at }",
            params,
            ScriptMutability::Mutable,
        )
        .unwrap();

        // Before consumption: canonical learning_events is empty, legacy bridge has 1 row.
        let before = store::list_all_learning_events(&db).unwrap();
        assert!(before.is_empty());

        // Consume
        let count = consume_completion_events(&db).unwrap();
        assert_eq!(count, 1, "should have consumed 1 event");

        // After consumption: canonical learning_events has 1 row, legacy bridge is empty.
        let after = store::list_all_learning_events(&db).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(
            after[0].event_type,
            LearningEventType::FalseCompletionDetected
        );

        // Verify the legacy bridge was drained.
        let count2 = consume_completion_events(&db).unwrap();
        assert_eq!(
            count2, 0,
            "legacy bridge should be empty after first consume"
        );
    }

    #[test]
    fn test_consume_incident_events_uses_linked_learning_event_id() {
        let db = test_db();
        create_false_completion_incidents(&db);
        insert_incident(&db, "inc-legacy-1", "le-linked-1");

        let consumed = consume_incident_events(&db).unwrap();
        assert_eq!(consumed, 1);

        let event = store::get_learning_event(&db, "le-linked-1")
            .unwrap()
            .expect("linked learning event must exist");
        assert_eq!(event.event_id, "le-linked-1");
        assert_eq!(event.event_type, LearningEventType::FalseCompletionDetected);
        assert_eq!(event.source_artifact_id, "inc-legacy-1");
        assert_eq!(event.signal["agent_key"], "agent-alpha");
        assert_eq!(event.signal["severity"], "High");
    }

    #[test]
    fn test_consume_incident_events_skips_already_canonical_event() {
        let db = test_db();
        create_false_completion_incidents(&db);
        insert_incident(&db, "inc-legacy-2", "le-linked-2");
        store::insert_learning_event(
            &db,
            &LearningEvent {
                event_id: "le-linked-2".into(),
                workspace_id: "default".into(),
                event_type: LearningEventType::FalseCompletionDetected,
                source_artifact_id: "inc-legacy-2".into(),
                outcome_artifact_id: None,
                signal: serde_json::json!({"already": true}),
                confidence: 1.0,
                provenance_record_id: String::new(),
                created_at: "2026-05-04T00:00:00Z".into(),
            },
        )
        .unwrap();

        let consumed = consume_incident_events(&db).unwrap();
        assert_eq!(consumed, 0);

        let events = store::list_all_learning_events(&db).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].signal, serde_json::json!({"already": true}));
    }
}
