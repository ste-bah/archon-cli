use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::Serialize;

use crate::types::{ReasoningClaim, ReasoningQualityEvent};

pub fn ensure_schema(db: &DbInstance) -> Result<()> {
    for script in [
        r#":create reasoning_claims {
            claim_id: String =>
            session_id: String,
            turn_number: Int,
            subject: String,
            entity_key: String,
            canonicalizer_version: String,
            canonical_text: String,
            claim_json: String,
            created_at: String,
        }"#,
        r#":create reasoning_quality_events {
            event_id: String =>
            session_id: String,
            turn_number: Int,
            claim_id: String,
            event_kind: String,
            subject: String,
            entity_key: String,
            event_json: String,
            created_at: String,
        }"#,
        r#":create reasoning_evidence_refs {
            event_id: String,
            evidence_id: String =>
            evidence_json: String,
        }"#,
        r#":create reasoning_session_summaries { session_id: String => summary_json: String }"#,
        r#":create reasoning_repeated_patterns { pattern_id: String => pattern_json: String }"#,
        r#":create reasoning_bridge_offsets { bridge_name: String => offset_json: String }"#,
        r#":create reasoning_critic_costs { cost_id: String => cost_json: String }"#,
        r#":create reasoning_schema_migrations { migration_id: String => migration_json: String }"#,
        r#":create reasoning_shadow_deltas { delta_id: String => delta_json: String }"#,
    ] {
        run_idempotent(db, script)?;
    }
    Ok(())
}

pub fn put_events(db: &DbInstance, events: &[ReasoningQualityEvent]) -> Result<usize> {
    for event in events {
        put_claim(db, event)?;
        put_event(db, event)?;
        put_evidence_refs(db, event)?;
    }
    Ok(events.len())
}

pub fn count_events(db: &DbInstance) -> Result<usize> {
    let result = db
        .run_script(
            "?[count(event_id)] := *reasoning_quality_events{event_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("reasoning event count failed: {e}"))?;
    Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
}

pub fn events_for_session(db: &DbInstance, session_id: &str) -> Result<Vec<ReasoningQualityEvent>> {
    let mut params = BTreeMap::new();
    params.insert("session_id".to_string(), DataValue::from(session_id));
    let result = db
        .run_script(
            "?[event_json] := *reasoning_quality_events{session_id: $session_id, event_json}",
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("reasoning event query failed: {e}"))?;

    let mut events = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        if let Some(json) = row[0].get_str() {
            events.push(serde_json::from_str(json)?);
        }
    }
    events.sort_by(
        |left: &ReasoningQualityEvent, right: &ReasoningQualityEvent| {
            left.turn_number
                .cmp(&right.turn_number)
                .then_with(|| left.event_id.cmp(&right.event_id))
        },
    );
    Ok(events)
}

pub fn recent_events(db: &DbInstance, limit: usize) -> Result<Vec<ReasoningQualityEvent>> {
    let result = db
        .run_script(
            "?[event_json] := *reasoning_quality_events{event_json}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("recent reasoning event query failed: {e}"))?;

    let mut events = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        if let Some(json) = row[0].get_str() {
            events.push(serde_json::from_str(json)?);
        }
    }
    events.sort_by(
        |left: &ReasoningQualityEvent, right: &ReasoningQualityEvent| {
            right
                .created_at
                .cmp(&left.created_at)
                .then_with(|| right.event_id.cmp(&left.event_id))
        },
    );
    events.truncate(limit);
    Ok(events)
}

pub fn put_schema_migration(db: &DbInstance, to_version: u32, dry_run: bool) -> Result<()> {
    let migration_id = format!(
        "rqmig-v{to_version}-{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S")
    );
    let payload = serde_json::json!({
        "migration_id": migration_id,
        "to_version": to_version,
        "dry_run": dry_run,
        "append_safe": true,
        "consumer_cutover_version": to_version,
        "created_at": chrono::Utc::now().to_rfc3339(),
    });
    if dry_run {
        return Ok(());
    }
    let mut params = BTreeMap::new();
    params.insert(
        "migration_id".to_string(),
        DataValue::from(migration_id.as_str()),
    );
    params.insert(
        "migration_json".to_string(),
        DataValue::from(payload.to_string().as_str()),
    );
    db.run_script(
        "?[migration_id, migration_json] <- [[$migration_id, $migration_json]]
         :put reasoning_schema_migrations { migration_id => migration_json }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("reasoning schema migration record failed: {e}"))?;
    Ok(())
}

fn put_claim(db: &DbInstance, event: &ReasoningQualityEvent) -> Result<()> {
    let claim = ReasoningClaim {
        claim_id: event.claim_id.clone(),
        canonicalizer_version: event.canonicalizer_version.clone(),
        canonical_text: event.canonical_text.clone(),
        subject: event.subject,
        entity_key: event.entity_key.clone(),
        confidence_signal: event.confidence_signal,
        turn_number: event.turn_number,
    };
    let mut params = BTreeMap::new();
    params.insert(
        "claim_id".to_string(),
        DataValue::from(event.claim_id.as_str()),
    );
    params.insert(
        "session_id".to_string(),
        DataValue::from(event.session_id.as_str()),
    );
    params.insert(
        "turn_number".to_string(),
        DataValue::from(event.turn_number as i64),
    );
    params.insert(
        "subject".to_string(),
        DataValue::from(enum_tag(&event.subject)?.as_str()),
    );
    params.insert(
        "entity_key".to_string(),
        DataValue::from(event.entity_key.as_str()),
    );
    params.insert(
        "canonicalizer_version".to_string(),
        DataValue::from(event.canonicalizer_version.as_str()),
    );
    params.insert(
        "canonical_text".to_string(),
        DataValue::from(event.canonical_text.as_str()),
    );
    params.insert(
        "claim_json".to_string(),
        DataValue::from(json_string(&claim)?.as_str()),
    );
    params.insert(
        "created_at".to_string(),
        DataValue::from(event.created_at.to_rfc3339().as_str()),
    );
    db.run_script(
        "?[claim_id, session_id, turn_number, subject, entity_key, canonicalizer_version, canonical_text, claim_json, created_at] <- \
         [[$claim_id, $session_id, $turn_number, $subject, $entity_key, $canonicalizer_version, $canonical_text, $claim_json, $created_at]]
         :put reasoning_claims { claim_id => session_id, turn_number, subject, entity_key, canonicalizer_version, canonical_text, claim_json, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("reasoning claim upsert failed: {e}"))?;
    Ok(())
}

fn put_event(db: &DbInstance, event: &ReasoningQualityEvent) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert(
        "event_id".to_string(),
        DataValue::from(event.event_id.as_str()),
    );
    params.insert(
        "session_id".to_string(),
        DataValue::from(event.session_id.as_str()),
    );
    params.insert(
        "turn_number".to_string(),
        DataValue::from(event.turn_number as i64),
    );
    params.insert(
        "claim_id".to_string(),
        DataValue::from(event.claim_id.as_str()),
    );
    params.insert(
        "event_kind".to_string(),
        DataValue::from(enum_tag(&event.event_kind)?.as_str()),
    );
    params.insert(
        "subject".to_string(),
        DataValue::from(enum_tag(&event.subject)?.as_str()),
    );
    params.insert(
        "entity_key".to_string(),
        DataValue::from(event.entity_key.as_str()),
    );
    params.insert(
        "event_json".to_string(),
        DataValue::from(json_string(event)?.as_str()),
    );
    params.insert(
        "created_at".to_string(),
        DataValue::from(event.created_at.to_rfc3339().as_str()),
    );
    db.run_script(
        "?[event_id, session_id, turn_number, claim_id, event_kind, subject, entity_key, event_json, created_at] <- \
         [[$event_id, $session_id, $turn_number, $claim_id, $event_kind, $subject, $entity_key, $event_json, $created_at]]
         :put reasoning_quality_events { event_id => session_id, turn_number, claim_id, event_kind, subject, entity_key, event_json, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("reasoning event upsert failed: {e}"))?;
    Ok(())
}

fn put_evidence_refs(db: &DbInstance, event: &ReasoningQualityEvent) -> Result<()> {
    for evidence in &event.evidence_refs {
        let mut params = BTreeMap::new();
        params.insert(
            "event_id".to_string(),
            DataValue::from(event.event_id.as_str()),
        );
        params.insert(
            "evidence_id".to_string(),
            DataValue::from(evidence.evidence_id.as_str()),
        );
        params.insert(
            "evidence_json".to_string(),
            DataValue::from(json_string(evidence)?.as_str()),
        );
        db.run_script(
            "?[event_id, evidence_id, evidence_json] <- [[$event_id, $evidence_id, $evidence_json]]
             :put reasoning_evidence_refs { event_id, evidence_id => evidence_json }",
            params,
            ScriptMutability::Mutable,
        )
        .map_err(|e| anyhow::anyhow!("reasoning evidence upsert failed: {e}"))?;
    }
    Ok(())
}

fn run_idempotent(db: &DbInstance, script: &str) -> Result<()> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already exists") || msg.contains("conflicts") {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "reasoning-quality schema creation failed: {msg}"
                ))
            }
        }
    }
}

fn json_string(value: &impl Serialize) -> Result<String> {
    serde_json::to_string(value).map_err(Into::into)
}

fn enum_tag(value: &impl Serialize) -> Result<String> {
    match serde_json::to_value(value)? {
        serde_json::Value::String(tag) => Ok(tag),
        other => Ok(other.to_string()),
    }
}
