use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::cozo_guard::run_script_guarded;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SandboxSessionRecord {
    pub sandbox_session_id: String,
    pub backend_kind: String,
    pub sandbox_profile_id: String,
    pub run_id: Option<String>,
    pub agent_type: Option<String>,
    pub backend_instance_id: Option<String>,
    pub workspace_mode: Option<String>,
    pub canonical_workspace: Option<String>,
    pub transport_kind: Option<String>,
    pub transport_endpoint_redacted: Option<String>,
    pub provider_injection_enabled: bool,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

impl SandboxSessionRecord {
    pub fn new(
        sandbox_session_id: impl Into<String>,
        backend_kind: impl Into<String>,
        sandbox_profile_id: impl Into<String>,
        status: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        let created_at = created_at.into();
        Self {
            sandbox_session_id: sandbox_session_id.into(),
            backend_kind: backend_kind.into(),
            sandbox_profile_id: sandbox_profile_id.into(),
            run_id: None,
            agent_type: None,
            backend_instance_id: None,
            workspace_mode: None,
            canonical_workspace: None,
            transport_kind: None,
            transport_endpoint_redacted: None,
            provider_injection_enabled: false,
            status: status.into(),
            updated_at: created_at.clone(),
            created_at,
        }
    }

    pub fn with_run_context(mut self, run_id: Option<String>, agent_type: Option<String>) -> Self {
        self.run_id = run_id;
        self.agent_type = agent_type;
        self
    }

    pub fn with_backend_instance(mut self, backend_instance_id: impl Into<String>) -> Self {
        self.backend_instance_id = Some(backend_instance_id.into());
        self
    }

    pub fn with_workspace(
        mut self,
        workspace_mode: Option<String>,
        canonical_workspace: Option<String>,
    ) -> Self {
        self.workspace_mode = workspace_mode;
        self.canonical_workspace = canonical_workspace;
        self
    }

    pub fn with_transport(
        mut self,
        transport_kind: Option<String>,
        transport_endpoint_redacted: Option<String>,
    ) -> Self {
        self.transport_kind = transport_kind;
        self.transport_endpoint_redacted = transport_endpoint_redacted;
        self
    }

    pub fn with_provider_injection_enabled(mut self) -> Self {
        self.provider_injection_enabled = true;
        self
    }
}

pub fn insert_sandbox_session(db: &DbInstance, session: &SandboxSessionRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert(
        "sid".into(),
        DataValue::from(session.sandbox_session_id.as_str()),
    );
    params.insert(
        "backend".into(),
        DataValue::from(session.backend_kind.as_str()),
    );
    params.insert(
        "profile".into(),
        DataValue::from(session.sandbox_profile_id.as_str()),
    );
    params.insert(
        "run".into(),
        DataValue::from(session.run_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "agent".into(),
        DataValue::from(session.agent_type.as_deref().unwrap_or("")),
    );
    params.insert(
        "instance".into(),
        DataValue::from(session.backend_instance_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "workspace_mode".into(),
        DataValue::from(session.workspace_mode.as_deref().unwrap_or("")),
    );
    params.insert(
        "canonical".into(),
        DataValue::from(session.canonical_workspace.as_deref().unwrap_or("")),
    );
    params.insert(
        "transport".into(),
        DataValue::from(session.transport_kind.as_deref().unwrap_or("")),
    );
    params.insert(
        "endpoint".into(),
        DataValue::from(session.transport_endpoint_redacted.as_deref().unwrap_or("")),
    );
    params.insert(
        "injection".into(),
        DataValue::from(session.provider_injection_enabled),
    );
    params.insert("status".into(), DataValue::from(session.status.as_str()));
    params.insert(
        "created".into(),
        DataValue::from(session.created_at.as_str()),
    );
    params.insert(
        "updated".into(),
        DataValue::from(session.updated_at.as_str()),
    );

    run_script_guarded(
        db,
        session_put_script(),
        params,
        ScriptMutability::Mutable,
        "insert sandbox_sessions failed",
    )?;
    Ok(())
}

pub fn get_sandbox_session(
    db: &DbInstance,
    sandbox_session_id: &str,
) -> Result<Option<SandboxSessionRecord>> {
    let mut params = BTreeMap::new();
    params.insert("sid".into(), DataValue::from(sandbox_session_id));
    let result = run_script_guarded(
        db,
        session_query("sandbox_session_id = $sid"),
        params,
        ScriptMutability::Immutable,
        "get sandbox_session failed",
    )?;
    Ok(result.rows.first().map(|row| row_to_session(row)))
}

pub fn list_sandbox_sessions_by_status(
    db: &DbInstance,
    status: &str,
) -> Result<Vec<SandboxSessionRecord>> {
    let mut params = BTreeMap::new();
    params.insert("status".into(), DataValue::from(status));
    let result = run_script_guarded(
        db,
        session_query("status = $status"),
        params,
        ScriptMutability::Immutable,
        "list sandbox_sessions failed",
    )?;
    let mut sessions: Vec<_> = result.rows.iter().map(|row| row_to_session(row)).collect();
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

pub fn list_sandbox_sessions(db: &DbInstance) -> Result<Vec<SandboxSessionRecord>> {
    let result = run_script_guarded(
        db,
        session_query("all"),
        Default::default(),
        ScriptMutability::Immutable,
        "list sandbox_sessions failed",
    )?;
    let mut sessions: Vec<_> = result.rows.iter().map(|row| row_to_session(row)).collect();
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

fn session_put_script() -> &'static str {
    "?[sandbox_session_id, backend_kind, sandbox_profile_id, run_id, \
     agent_type, backend_instance_id, workspace_mode, canonical_workspace, \
     transport_kind, transport_endpoint_redacted, provider_injection_enabled, \
     status, created_at, updated_at] <- [[$sid, $backend, $profile, $run, \
     $agent, $instance, $workspace_mode, $canonical, $transport, $endpoint, \
     $injection, $status, $created, $updated]] :put sandbox_sessions { \
     sandbox_session_id => backend_kind, sandbox_profile_id, run_id, \
     agent_type, backend_instance_id, workspace_mode, canonical_workspace, \
     transport_kind, transport_endpoint_redacted, provider_injection_enabled, \
     status, created_at, updated_at }"
}

fn session_query(predicate: &'static str) -> &'static str {
    match predicate {
        "sandbox_session_id = $sid" => {
            "?[sandbox_session_id, backend_kind, sandbox_profile_id, run_id, \
             agent_type, backend_instance_id, workspace_mode, canonical_workspace, \
             transport_kind, transport_endpoint_redacted, provider_injection_enabled, \
             status, created_at, updated_at] := *sandbox_sessions{ \
             sandbox_session_id, backend_kind, sandbox_profile_id, run_id, \
             agent_type, backend_instance_id, workspace_mode, canonical_workspace, \
             transport_kind, transport_endpoint_redacted, provider_injection_enabled, \
             status, created_at, updated_at}, sandbox_session_id = $sid"
        }
        "all" => {
            "?[sandbox_session_id, backend_kind, sandbox_profile_id, run_id, \
             agent_type, backend_instance_id, workspace_mode, canonical_workspace, \
             transport_kind, transport_endpoint_redacted, provider_injection_enabled, \
             status, created_at, updated_at] := *sandbox_sessions{ \
             sandbox_session_id, backend_kind, sandbox_profile_id, run_id, \
             agent_type, backend_instance_id, workspace_mode, canonical_workspace, \
             transport_kind, transport_endpoint_redacted, provider_injection_enabled, \
             status, created_at, updated_at}"
        }
        _ => {
            "?[sandbox_session_id, backend_kind, sandbox_profile_id, run_id, \
             agent_type, backend_instance_id, workspace_mode, canonical_workspace, \
             transport_kind, transport_endpoint_redacted, provider_injection_enabled, \
             status, created_at, updated_at] := *sandbox_sessions{ \
             sandbox_session_id, backend_kind, sandbox_profile_id, run_id, \
             agent_type, backend_instance_id, workspace_mode, canonical_workspace, \
             transport_kind, transport_endpoint_redacted, provider_injection_enabled, \
             status, created_at, updated_at}, status = $status"
        }
    }
}

fn row_to_session(row: &[DataValue]) -> SandboxSessionRecord {
    SandboxSessionRecord {
        sandbox_session_id: str_col(row, 0).to_string(),
        backend_kind: str_col(row, 1).to_string(),
        sandbox_profile_id: str_col(row, 2).to_string(),
        run_id: non_empty(str_col(row, 3)),
        agent_type: non_empty(str_col(row, 4)),
        backend_instance_id: non_empty(str_col(row, 5)),
        workspace_mode: non_empty(str_col(row, 6)),
        canonical_workspace: non_empty(str_col(row, 7)),
        transport_kind: non_empty(str_col(row, 8)),
        transport_endpoint_redacted: non_empty(str_col(row, 9)),
        provider_injection_enabled: row[10].get_bool().unwrap_or(false),
        status: str_col(row, 11).to_string(),
        created_at: str_col(row, 12).to_string(),
        updated_at: str_col(row, 13).to_string(),
    }
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
        let path = format!("/tmp/test-sandbox-sessions-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn sandbox_session_roundtrips_with_provider_injection_disabled_by_default() {
        let db = test_db();
        let session = SandboxSessionRecord::new(
            "sandbox-session-1",
            "openshell",
            "sandbox-profile-1",
            "active",
            "2026-05-08T12:00:00Z",
        )
        .with_workspace(Some("mirror".into()), Some("local".into()))
        .with_transport(Some("ssh".into()), Some("redacted-gateway".into()));

        insert_sandbox_session(&db, &session).unwrap();
        let restored = get_sandbox_session(&db, "sandbox-session-1")
            .unwrap()
            .unwrap();
        let listed = list_sandbox_sessions_by_status(&db, "active").unwrap();
        let all = list_sandbox_sessions(&db).unwrap();

        assert_eq!(restored.backend_kind, "openshell");
        assert_eq!(restored.workspace_mode.as_deref(), Some("mirror"));
        assert_eq!(
            restored.transport_endpoint_redacted.as_deref(),
            Some("redacted-gateway")
        );
        assert!(!restored.provider_injection_enabled);
        assert_eq!(listed.len(), 1);
        assert_eq!(all.len(), 1);
    }
}
