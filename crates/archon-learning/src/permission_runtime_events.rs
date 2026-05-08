use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PermissionRuntimeEventRecord {
    pub event_id: String,
    pub session_id: Option<String>,
    pub run_id: Option<String>,
    pub agent_type: Option<String>,
    pub tool_name: String,
    pub permission_mode: String,
    pub decision: String,
    pub reason_code: Option<String>,
    pub rule_name: Option<String>,
    pub sandbox_backend: Option<String>,
    pub raw_redacted_json: serde_json::Value,
    pub created_at: String,
}

impl PermissionRuntimeEventRecord {
    pub fn new(
        event_id: impl Into<String>,
        tool_name: impl Into<String>,
        permission_mode: impl Into<String>,
        decision: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            session_id: None,
            run_id: None,
            agent_type: None,
            tool_name: tool_name.into(),
            permission_mode: permission_mode.into(),
            decision: decision.into(),
            reason_code: None,
            rule_name: None,
            sandbox_backend: None,
            raw_redacted_json: serde_json::json!({}),
            created_at: created_at.into(),
        }
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_run_context(mut self, run_id: Option<String>, agent_type: Option<String>) -> Self {
        self.run_id = run_id;
        self.agent_type = agent_type;
        self
    }

    pub fn with_policy_context(
        mut self,
        reason_code: Option<String>,
        rule_name: Option<String>,
        sandbox_backend: Option<String>,
    ) -> Self {
        self.reason_code = reason_code;
        self.rule_name = rule_name;
        self.sandbox_backend = sandbox_backend;
        self
    }

    pub fn with_raw_redacted_json(mut self, raw_redacted_json: serde_json::Value) -> Self {
        self.raw_redacted_json = raw_redacted_json;
        self
    }
}

pub fn insert_permission_runtime_event(
    db: &DbInstance,
    event: &PermissionRuntimeEventRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(event.event_id.as_str()));
    params.insert(
        "session".into(),
        DataValue::from(event.session_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "run".into(),
        DataValue::from(event.run_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "agent".into(),
        DataValue::from(event.agent_type.as_deref().unwrap_or("")),
    );
    params.insert("tool".into(), DataValue::from(event.tool_name.as_str()));
    params.insert(
        "mode".into(),
        DataValue::from(event.permission_mode.as_str()),
    );
    params.insert("decision".into(), DataValue::from(event.decision.as_str()));
    params.insert(
        "reason".into(),
        DataValue::from(event.reason_code.as_deref().unwrap_or("")),
    );
    params.insert(
        "rule".into(),
        DataValue::from(event.rule_name.as_deref().unwrap_or("")),
    );
    params.insert(
        "sandbox".into(),
        DataValue::from(event.sandbox_backend.as_deref().unwrap_or("")),
    );
    params.insert(
        "raw".into(),
        DataValue::from(event.raw_redacted_json.to_string().as_str()),
    );
    params.insert("created".into(), DataValue::from(event.created_at.as_str()));

    db.run_script(
        permission_event_put_script(),
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert permission_runtime_events failed: {e}"))?;
    Ok(())
}

pub fn get_permission_runtime_event(
    db: &DbInstance,
    event_id: &str,
) -> Result<Option<PermissionRuntimeEventRecord>> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(event_id));
    let result = db
        .run_script(
            permission_event_query("event_id = $eid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get permission_runtime_event failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_permission_event(row)))
}

pub fn list_permission_runtime_events_by_session(
    db: &DbInstance,
    session_id: &str,
) -> Result<Vec<PermissionRuntimeEventRecord>> {
    let mut params = BTreeMap::new();
    params.insert("session".into(), DataValue::from(session_id));
    let result = db
        .run_script(
            permission_event_query("session_id = $session"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list permission_runtime_events by session failed: {e}"))?;
    Ok(sorted(
        result.rows.iter().map(|row| row_to_permission_event(row)),
    ))
}

pub fn list_permission_runtime_events_by_decision(
    db: &DbInstance,
    decision: &str,
) -> Result<Vec<PermissionRuntimeEventRecord>> {
    let mut params = BTreeMap::new();
    params.insert("decision".into(), DataValue::from(decision));
    let result = db
        .run_script(
            permission_event_query("decision = $decision"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list permission_runtime_events by decision failed: {e}"))?;
    Ok(sorted(
        result.rows.iter().map(|row| row_to_permission_event(row)),
    ))
}

fn permission_event_put_script() -> &'static str {
    "?[event_id, session_id, run_id, agent_type, tool_name, permission_mode, \
     decision, reason_code, rule_name, sandbox_backend, raw_redacted_json, \
     created_at] <- [[$eid, $session, $run, $agent, $tool, $mode, \
     $decision, $reason, $rule, $sandbox, $raw, $created]] \
     :put permission_runtime_events { event_id => session_id, run_id, \
     agent_type, tool_name, permission_mode, decision, reason_code, \
     rule_name, sandbox_backend, raw_redacted_json, created_at }"
}

fn permission_event_query(predicate: &'static str) -> &'static str {
    match predicate {
        "event_id = $eid" => {
            "?[event_id, session_id, run_id, agent_type, tool_name, \
             permission_mode, decision, reason_code, rule_name, \
             sandbox_backend, raw_redacted_json, created_at] := \
             *permission_runtime_events{event_id, session_id, run_id, \
             agent_type, tool_name, permission_mode, decision, reason_code, \
             rule_name, sandbox_backend, raw_redacted_json, created_at}, \
             event_id = $eid"
        }
        "session_id = $session" => {
            "?[event_id, session_id, run_id, agent_type, tool_name, \
             permission_mode, decision, reason_code, rule_name, \
             sandbox_backend, raw_redacted_json, created_at] := \
             *permission_runtime_events{event_id, session_id, run_id, \
             agent_type, tool_name, permission_mode, decision, reason_code, \
             rule_name, sandbox_backend, raw_redacted_json, created_at}, \
             session_id = $session"
        }
        _ => {
            "?[event_id, session_id, run_id, agent_type, tool_name, \
             permission_mode, decision, reason_code, rule_name, \
             sandbox_backend, raw_redacted_json, created_at] := \
             *permission_runtime_events{event_id, session_id, run_id, \
             agent_type, tool_name, permission_mode, decision, reason_code, \
             rule_name, sandbox_backend, raw_redacted_json, created_at}, \
             decision = $decision"
        }
    }
}

fn row_to_permission_event(row: &[DataValue]) -> PermissionRuntimeEventRecord {
    PermissionRuntimeEventRecord {
        event_id: str_col(row, 0).to_string(),
        session_id: non_empty(str_col(row, 1)),
        run_id: non_empty(str_col(row, 2)),
        agent_type: non_empty(str_col(row, 3)),
        tool_name: str_col(row, 4).to_string(),
        permission_mode: str_col(row, 5).to_string(),
        decision: str_col(row, 6).to_string(),
        reason_code: non_empty(str_col(row, 7)),
        rule_name: non_empty(str_col(row, 8)),
        sandbox_backend: non_empty(str_col(row, 9)),
        raw_redacted_json: serde_json::from_str(str_col(row, 10))
            .unwrap_or_else(|_| serde_json::json!({})),
        created_at: str_col(row, 11).to_string(),
    }
}

fn sorted(
    records: impl Iterator<Item = PermissionRuntimeEventRecord>,
) -> Vec<PermissionRuntimeEventRecord> {
    let mut records: Vec<_> = records.collect();
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    records
}

fn str_col(row: &[DataValue], index: usize) -> &str {
    row[index].get_str().unwrap_or("")
}

fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        let path = format!(
            "/tmp/test-permission-runtime-events-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn permission_runtime_event_roundtrips() {
        let db = test_db();
        let event = PermissionRuntimeEventRecord::new(
            "permission-event-1",
            "Bash",
            "ask",
            "denied",
            "2026-05-08T12:00:00Z",
        )
        .with_session("session-1")
        .with_policy_context(
            Some("user_denied_or_timeout".to_string()),
            Some("always_deny".to_string()),
            Some("logical".to_string()),
        )
        .with_raw_redacted_json(serde_json::json!({"payload_redacted": true}));

        insert_permission_runtime_event(&db, &event).unwrap();
        let restored = get_permission_runtime_event(&db, "permission-event-1")
            .unwrap()
            .unwrap();

        assert_eq!(restored.session_id.as_deref(), Some("session-1"));
        assert_eq!(restored.tool_name, "Bash");
        assert_eq!(restored.decision, "denied");
        assert_eq!(
            restored.reason_code.as_deref(),
            Some("user_denied_or_timeout")
        );
        assert_eq!(restored.raw_redacted_json["payload_redacted"], true);
    }

    #[test]
    fn permission_runtime_events_list_by_session_and_decision() {
        let db = test_db();
        insert_permission_runtime_event(
            &db,
            &PermissionRuntimeEventRecord::new(
                "permission-event-1",
                "Write",
                "ask",
                "denied",
                "2026-05-08T12:00:00Z",
            )
            .with_session("session-1"),
        )
        .unwrap();
        insert_permission_runtime_event(
            &db,
            &PermissionRuntimeEventRecord::new(
                "permission-event-2",
                "Read",
                "ask",
                "granted",
                "2026-05-08T12:01:00Z",
            )
            .with_session("session-1"),
        )
        .unwrap();

        let by_session = list_permission_runtime_events_by_session(&db, "session-1").unwrap();
        let denied = list_permission_runtime_events_by_decision(&db, "denied").unwrap();

        assert_eq!(by_session.len(), 2);
        assert_eq!(by_session[0].event_id, "permission-event-2");
        assert_eq!(denied.len(), 1);
        assert_eq!(denied[0].tool_name, "Write");
    }
}
