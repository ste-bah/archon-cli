use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SandboxRuntimeEventRecord {
    pub event_id: String,
    pub backend_kind: String,
    pub backend_instance_id: Option<String>,
    pub agent_type: Option<String>,
    pub run_id: Option<String>,
    pub tool_name: Option<String>,
    pub decision: String,
    pub reason_code: Option<String>,
    pub sandbox_profile_id: Option<String>,
    pub workspace_mode: Option<String>,
    pub network_mode: Option<String>,
    pub workspace_mount_mode: Option<String>,
    pub redacted_context_json: serde_json::Value,
    pub created_at: String,
}

impl SandboxRuntimeEventRecord {
    pub fn new(
        event_id: impl Into<String>,
        backend_kind: impl Into<String>,
        decision: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            backend_kind: backend_kind.into(),
            backend_instance_id: None,
            agent_type: None,
            run_id: None,
            tool_name: None,
            decision: decision.into(),
            reason_code: None,
            sandbox_profile_id: None,
            workspace_mode: None,
            network_mode: None,
            workspace_mount_mode: None,
            redacted_context_json: serde_json::json!({}),
            created_at: created_at.into(),
        }
    }

    pub fn with_run_context(mut self, run_id: Option<String>, agent_type: Option<String>) -> Self {
        self.run_id = run_id;
        self.agent_type = agent_type;
        self
    }

    pub fn with_tool(mut self, tool_name: impl Into<String>) -> Self {
        self.tool_name = Some(tool_name.into());
        self
    }

    pub fn with_backend_instance(mut self, backend_instance_id: impl Into<String>) -> Self {
        self.backend_instance_id = Some(backend_instance_id.into());
        self
    }

    pub fn with_policy(
        mut self,
        reason_code: Option<String>,
        sandbox_profile_id: Option<String>,
        workspace_mode: Option<String>,
        network_mode: Option<String>,
        workspace_mount_mode: Option<String>,
    ) -> Self {
        self.reason_code = reason_code;
        self.sandbox_profile_id = sandbox_profile_id;
        self.workspace_mode = workspace_mode;
        self.network_mode = network_mode;
        self.workspace_mount_mode = workspace_mount_mode;
        self
    }

    pub fn with_redacted_context(mut self, context: serde_json::Value) -> Self {
        self.redacted_context_json = context;
        self
    }
}

pub fn insert_sandbox_runtime_event(
    db: &DbInstance,
    event: &SandboxRuntimeEventRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(event.event_id.as_str()));
    params.insert(
        "backend".into(),
        DataValue::from(event.backend_kind.as_str()),
    );
    params.insert(
        "instance".into(),
        DataValue::from(event.backend_instance_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "agent".into(),
        DataValue::from(event.agent_type.as_deref().unwrap_or("")),
    );
    params.insert(
        "run".into(),
        DataValue::from(event.run_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "tool".into(),
        DataValue::from(event.tool_name.as_deref().unwrap_or("")),
    );
    params.insert("decision".into(), DataValue::from(event.decision.as_str()));
    params.insert(
        "reason".into(),
        DataValue::from(event.reason_code.as_deref().unwrap_or("")),
    );
    params.insert(
        "profile".into(),
        DataValue::from(event.sandbox_profile_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "workspace".into(),
        DataValue::from(event.workspace_mode.as_deref().unwrap_or("")),
    );
    params.insert(
        "network".into(),
        DataValue::from(event.network_mode.as_deref().unwrap_or("")),
    );
    params.insert(
        "mount".into(),
        DataValue::from(event.workspace_mount_mode.as_deref().unwrap_or("")),
    );
    params.insert(
        "context".into(),
        DataValue::from(event.redacted_context_json.to_string().as_str()),
    );
    params.insert("created".into(), DataValue::from(event.created_at.as_str()));

    db.run_script(event_put_script(), params, ScriptMutability::Mutable)
        .map_err(|e| anyhow::anyhow!("insert sandbox_runtime_events failed: {e}"))?;
    Ok(())
}

pub fn get_sandbox_runtime_event(
    db: &DbInstance,
    event_id: &str,
) -> Result<Option<SandboxRuntimeEventRecord>> {
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(event_id));
    let result = db
        .run_script(
            event_query("event_id = $eid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get sandbox_runtime_event failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_event(row)))
}

pub fn list_sandbox_runtime_events_by_backend(
    db: &DbInstance,
    backend_kind: &str,
) -> Result<Vec<SandboxRuntimeEventRecord>> {
    let mut params = BTreeMap::new();
    params.insert("backend".into(), DataValue::from(backend_kind));
    let result = db
        .run_script(
            event_query("backend_kind = $backend"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list sandbox_runtime_events failed: {e}"))?;
    Ok(sorted(result.rows.iter().map(|row| row_to_event(row))))
}

fn event_put_script() -> &'static str {
    "?[event_id, backend_kind, backend_instance_id, agent_type, run_id, \
     tool_name, decision, reason_code, sandbox_profile_id, workspace_mode, \
     network_mode, workspace_mount_mode, redacted_context_json, created_at] \
     <- [[$eid, $backend, $instance, $agent, $run, $tool, $decision, \
     $reason, $profile, $workspace, $network, $mount, $context, $created]] \
     :put sandbox_runtime_events { event_id => backend_kind, \
     backend_instance_id, agent_type, run_id, tool_name, decision, \
     reason_code, sandbox_profile_id, workspace_mode, network_mode, \
     workspace_mount_mode, redacted_context_json, created_at }"
}

fn event_query(predicate: &'static str) -> &'static str {
    match predicate {
        "event_id = $eid" => {
            "?[event_id, backend_kind, backend_instance_id, agent_type, run_id, \
             tool_name, decision, reason_code, sandbox_profile_id, workspace_mode, \
             network_mode, workspace_mount_mode, redacted_context_json, created_at] := \
             *sandbox_runtime_events{event_id, backend_kind, backend_instance_id, \
             agent_type, run_id, tool_name, decision, reason_code, sandbox_profile_id, \
             workspace_mode, network_mode, workspace_mount_mode, redacted_context_json, \
             created_at}, event_id = $eid"
        }
        _ => {
            "?[event_id, backend_kind, backend_instance_id, agent_type, run_id, \
             tool_name, decision, reason_code, sandbox_profile_id, workspace_mode, \
             network_mode, workspace_mount_mode, redacted_context_json, created_at] := \
             *sandbox_runtime_events{event_id, backend_kind, backend_instance_id, \
             agent_type, run_id, tool_name, decision, reason_code, sandbox_profile_id, \
             workspace_mode, network_mode, workspace_mount_mode, redacted_context_json, \
             created_at}, backend_kind = $backend"
        }
    }
}

fn row_to_event(row: &[DataValue]) -> SandboxRuntimeEventRecord {
    SandboxRuntimeEventRecord {
        event_id: str_col(row, 0).to_string(),
        backend_kind: str_col(row, 1).to_string(),
        backend_instance_id: non_empty(str_col(row, 2)),
        agent_type: non_empty(str_col(row, 3)),
        run_id: non_empty(str_col(row, 4)),
        tool_name: non_empty(str_col(row, 5)),
        decision: str_col(row, 6).to_string(),
        reason_code: non_empty(str_col(row, 7)),
        sandbox_profile_id: non_empty(str_col(row, 8)),
        workspace_mode: non_empty(str_col(row, 9)),
        network_mode: non_empty(str_col(row, 10)),
        workspace_mount_mode: non_empty(str_col(row, 11)),
        redacted_context_json: serde_json::from_str(str_col(row, 12))
            .unwrap_or_else(|_| serde_json::json!({})),
        created_at: str_col(row, 13).to_string(),
    }
}

fn sorted(
    records: impl Iterator<Item = SandboxRuntimeEventRecord>,
) -> Vec<SandboxRuntimeEventRecord> {
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
        let path = format!("/tmp/test-sandbox-events-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn sandbox_runtime_event_roundtrips() {
        let db = test_db();
        let event = SandboxRuntimeEventRecord::new(
            "sandbox-event-1",
            "openshell",
            "route_to_sandbox",
            "2026-05-08T12:00:00Z",
        )
        .with_tool("Bash")
        .with_policy(
            Some("risky_tool".into()),
            Some("sandbox-profile-1".into()),
            Some("mirror".into()),
            Some("disabled".into()),
            Some("rw".into()),
        )
        .with_redacted_context(serde_json::json!({"command": "redacted"}));

        insert_sandbox_runtime_event(&db, &event).unwrap();
        let restored = get_sandbox_runtime_event(&db, "sandbox-event-1")
            .unwrap()
            .unwrap();
        let listed = list_sandbox_runtime_events_by_backend(&db, "openshell").unwrap();

        assert_eq!(restored.backend_kind, "openshell");
        assert_eq!(restored.tool_name.as_deref(), Some("Bash"));
        assert_eq!(restored.workspace_mode.as_deref(), Some("mirror"));
        assert_eq!(restored.redacted_context_json["command"], "redacted");
        assert_eq!(listed.len(), 1);
    }
}
