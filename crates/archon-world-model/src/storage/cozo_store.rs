//! Cozo relation for indexed world-model trace rows.

use std::collections::BTreeMap;
use std::collections::HashSet;

use anyhow::Result;
use chrono::{DateTime, Utc};
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::Serialize;

use crate::schema::WorldTraceRow;
use crate::trace::ColdStartStats;

pub fn ensure_schema(db: &DbInstance) -> Result<()> {
    run_idempotent(
        db,
        r#":create world_trace_rows {
            row_id: String =>
            session_id: String,
            source: String,
            action_kind: String,
            provider: String,
            model: String,
            agent: String,
            labels_json: String,
            scalar_json: String,
            evidence_json: String,
            row_json: String,
            created_at: String,
        }"#,
    )
}

pub fn put_rows(db: &DbInstance, rows: &[WorldTraceRow]) -> Result<usize> {
    for row in rows {
        put_row(db, row)?;
    }
    Ok(rows.len())
}

pub fn count_rows(db: &DbInstance) -> Result<usize> {
    let result = db
        .run_script(
            "?[count(row_id)] := *world_trace_rows{row_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("world trace row count failed: {e}"))?;
    Ok(result.rows[0][0].get_int().unwrap_or(0) as usize)
}

pub fn all_rows(db: &DbInstance) -> Result<Vec<WorldTraceRow>> {
    let result = db
        .run_script(
            "?[row_json] := *world_trace_rows{row_json}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("world trace row load failed: {e}"))?;

    let mut rows = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        if let Some(row_json) = row[0].get_str() {
            rows.push(serde_json::from_str(row_json)?);
        }
    }
    rows.sort_by(|left: &WorldTraceRow, right: &WorldTraceRow| {
        left.session_id
            .cmp(&right.session_id)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.row_id.cmp(&right.row_id))
    });
    Ok(rows)
}

pub fn cold_start_stats(db: &DbInstance) -> Result<ColdStartStats> {
    let result = db
        .run_script(
            "?[session_id, created_at] := *world_trace_rows{session_id, created_at}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("world trace stats query failed: {e}"))?;

    let mut sessions = HashSet::new();
    let mut observed_days = HashSet::new();
    for row in &result.rows {
        if let Some(session_id) = row[0].get_str() {
            sessions.insert(session_id.to_string());
        }
        if let Some(created_at) = row[1].get_str() {
            if let Ok(date_time) = DateTime::parse_from_rfc3339(created_at) {
                observed_days.insert(date_time.with_timezone(&Utc).date_naive());
            }
        }
    }

    Ok(ColdStartStats {
        rows: result.rows.len() as u64,
        sessions: sessions.len() as u64,
        observed_days: observed_days.len() as u64,
    })
}

fn put_row(db: &DbInstance, row: &WorldTraceRow) -> Result<()> {
    let source = enum_tag(&row.source)?;
    let action_kind = enum_tag(&row.action_kind)?;
    let labels_json = json_string(&row.labels)?;
    let scalar_json = json_string(&row.scalar_features)?;
    let evidence_json = json_string(&row.evidence_refs)?;
    let row_json = json_string(row)?;
    let created_at = row.created_at.to_rfc3339();

    let mut params = BTreeMap::new();
    params.insert("row_id".to_string(), DataValue::from(row.row_id.as_str()));
    params.insert(
        "session_id".to_string(),
        DataValue::from(row.session_id.as_str()),
    );
    params.insert("source".to_string(), DataValue::from(source.as_str()));
    params.insert(
        "action_kind".to_string(),
        DataValue::from(action_kind.as_str()),
    );
    params.insert(
        "provider".to_string(),
        DataValue::from(opt_str(&row.provider)),
    );
    params.insert("model".to_string(), DataValue::from(opt_str(&row.model)));
    params.insert("agent".to_string(), DataValue::from(opt_str(&row.agent)));
    params.insert(
        "labels_json".to_string(),
        DataValue::from(labels_json.as_str()),
    );
    params.insert(
        "scalar_json".to_string(),
        DataValue::from(scalar_json.as_str()),
    );
    params.insert(
        "evidence_json".to_string(),
        DataValue::from(evidence_json.as_str()),
    );
    params.insert("row_json".to_string(), DataValue::from(row_json.as_str()));
    params.insert(
        "created_at".to_string(),
        DataValue::from(created_at.as_str()),
    );

    db.run_script(
        "?[row_id, session_id, source, action_kind, provider, model, agent, labels_json, scalar_json, evidence_json, row_json, created_at] <- \
         [[$row_id, $session_id, $source, $action_kind, $provider, $model, $agent, $labels_json, $scalar_json, $evidence_json, $row_json, $created_at]]
         :put world_trace_rows { row_id => session_id, source, action_kind, provider, model, agent, labels_json, scalar_json, evidence_json, row_json, created_at }",
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("world trace row upsert failed: {e}"))?;

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
                Err(anyhow::anyhow!("world model schema creation failed: {msg}"))
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

fn opt_str(value: &Option<String>) -> &str {
    value.as_deref().unwrap_or("")
}

#[cfg(test)]
mod tests {
    use crate::schema::{WorldActionKind, WorldTraceRow};

    use super::*;

    fn test_db() -> DbInstance {
        let path = format!("/tmp/archon-world-model-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    #[test]
    fn upserts_trace_rows_by_row_id() {
        let db = test_db();
        ensure_schema(&db).unwrap();

        let row =
            WorldTraceRow::new("session-1", WorldActionKind::ToolCall).with_row_id("stable-row");
        put_rows(&db, &[row.clone()]).unwrap();
        put_rows(&db, &[row]).unwrap();

        assert_eq!(count_rows(&db).unwrap(), 1);
        let stats = cold_start_stats(&db).unwrap();
        assert_eq!(stats.rows, 1);
        assert_eq!(stats.sessions, 1);
        assert_eq!(stats.observed_days, 1);
    }

    #[test]
    fn loads_rows_sorted_by_session_and_time() {
        let db = test_db();
        ensure_schema(&db).unwrap();

        let first = WorldTraceRow::new("session-1", WorldActionKind::ToolCall).with_row_id("a");
        let second = WorldTraceRow::new("session-1", WorldActionKind::Retry).with_row_id("b");
        put_rows(&db, &[second.clone(), first.clone()]).unwrap();

        let rows = all_rows(&db).unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].row_id, first.row_id);
        assert_eq!(rows[1].row_id, second.row_id);
    }
}
