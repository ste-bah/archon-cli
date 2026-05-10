use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SandboxProfileRecord {
    pub sandbox_profile_id: String,
    pub backend_kind: String,
    pub display_name: Option<String>,
    pub default_network_mode: Option<String>,
    pub workspace_mount_mode: Option<String>,
    pub writable_paths: Vec<String>,
    pub env_allowlist: Vec<String>,
    pub resource_limits_json: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

impl SandboxProfileRecord {
    pub fn new(
        sandbox_profile_id: impl Into<String>,
        backend_kind: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        let created_at = created_at.into();
        Self {
            sandbox_profile_id: sandbox_profile_id.into(),
            backend_kind: backend_kind.into(),
            display_name: None,
            default_network_mode: None,
            workspace_mount_mode: None,
            writable_paths: Vec::new(),
            env_allowlist: Vec::new(),
            resource_limits_json: serde_json::json!({}),
            updated_at: created_at.clone(),
            created_at,
        }
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = Some(display_name.into());
        self
    }

    pub fn with_policy(
        mut self,
        network_mode: Option<String>,
        workspace_mount_mode: Option<String>,
    ) -> Self {
        self.default_network_mode = network_mode;
        self.workspace_mount_mode = workspace_mount_mode;
        self
    }

    pub fn with_writable_path(mut self, path: impl Into<String>) -> Self {
        push_unique(&mut self.writable_paths, path.into());
        self
    }

    pub fn with_env_allow(mut self, name: impl Into<String>) -> Self {
        push_unique(&mut self.env_allowlist, name.into());
        self
    }

    pub fn with_resource_limits(mut self, limits: serde_json::Value) -> Self {
        self.resource_limits_json = limits;
        self
    }
}

pub fn insert_sandbox_profile(db: &DbInstance, profile: &SandboxProfileRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert(
        "sid".into(),
        DataValue::from(profile.sandbox_profile_id.as_str()),
    );
    params.insert(
        "backend".into(),
        DataValue::from(profile.backend_kind.as_str()),
    );
    params.insert(
        "display".into(),
        DataValue::from(profile.display_name.as_deref().unwrap_or("")),
    );
    params.insert(
        "network".into(),
        DataValue::from(profile.default_network_mode.as_deref().unwrap_or("")),
    );
    params.insert(
        "mount".into(),
        DataValue::from(profile.workspace_mount_mode.as_deref().unwrap_or("")),
    );
    params.insert(
        "writable".into(),
        DataValue::from(serde_json::to_string(&profile.writable_paths)?.as_str()),
    );
    params.insert(
        "env".into(),
        DataValue::from(serde_json::to_string(&profile.env_allowlist)?.as_str()),
    );
    params.insert(
        "limits".into(),
        DataValue::from(profile.resource_limits_json.to_string().as_str()),
    );
    params.insert(
        "created".into(),
        DataValue::from(profile.created_at.as_str()),
    );
    params.insert(
        "updated".into(),
        DataValue::from(profile.updated_at.as_str()),
    );

    db.run_script(profile_put_script(), params, ScriptMutability::Mutable)
        .map_err(|e| anyhow::anyhow!("insert sandbox_profiles failed: {e}"))?;
    Ok(())
}

pub fn get_sandbox_profile(
    db: &DbInstance,
    sandbox_profile_id: &str,
) -> Result<Option<SandboxProfileRecord>> {
    let mut params = BTreeMap::new();
    params.insert("sid".into(), DataValue::from(sandbox_profile_id));
    let result = db
        .run_script(
            profile_query("sandbox_profile_id = $sid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get sandbox_profile failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_profile(row)))
}

pub fn list_sandbox_profiles_by_backend(
    db: &DbInstance,
    backend_kind: &str,
) -> Result<Vec<SandboxProfileRecord>> {
    let mut params = BTreeMap::new();
    params.insert("backend".into(), DataValue::from(backend_kind));
    let result = db
        .run_script(
            profile_query("backend_kind = $backend"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list sandbox_profiles failed: {e}"))?;
    let mut profiles: Vec<_> = result.rows.iter().map(|row| row_to_profile(row)).collect();
    profiles.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(profiles)
}

fn profile_put_script() -> &'static str {
    "?[sandbox_profile_id, backend_kind, display_name, default_network_mode, \
     workspace_mount_mode, writable_paths_json, env_allowlist_json, \
     resource_limits_json, created_at, updated_at] <- [[$sid, $backend, \
     $display, $network, $mount, $writable, $env, $limits, $created, \
     $updated]] :put sandbox_profiles { sandbox_profile_id => backend_kind, \
     display_name, default_network_mode, workspace_mount_mode, \
     writable_paths_json, env_allowlist_json, resource_limits_json, \
     created_at, updated_at }"
}

fn profile_query(predicate: &'static str) -> &'static str {
    match predicate {
        "sandbox_profile_id = $sid" => {
            "?[sandbox_profile_id, backend_kind, display_name, default_network_mode, \
             workspace_mount_mode, writable_paths_json, env_allowlist_json, \
             resource_limits_json, created_at, updated_at] := *sandbox_profiles{ \
             sandbox_profile_id, backend_kind, display_name, default_network_mode, \
             workspace_mount_mode, writable_paths_json, env_allowlist_json, \
             resource_limits_json, created_at, updated_at}, sandbox_profile_id = $sid"
        }
        _ => {
            "?[sandbox_profile_id, backend_kind, display_name, default_network_mode, \
             workspace_mount_mode, writable_paths_json, env_allowlist_json, \
             resource_limits_json, created_at, updated_at] := *sandbox_profiles{ \
             sandbox_profile_id, backend_kind, display_name, default_network_mode, \
             workspace_mount_mode, writable_paths_json, env_allowlist_json, \
             resource_limits_json, created_at, updated_at}, backend_kind = $backend"
        }
    }
}

fn row_to_profile(row: &[DataValue]) -> SandboxProfileRecord {
    SandboxProfileRecord {
        sandbox_profile_id: str_col(row, 0).to_string(),
        backend_kind: str_col(row, 1).to_string(),
        display_name: non_empty(str_col(row, 2)),
        default_network_mode: non_empty(str_col(row, 3)),
        workspace_mount_mode: non_empty(str_col(row, 4)),
        writable_paths: serde_json::from_str(str_col(row, 5)).unwrap_or_default(),
        env_allowlist: serde_json::from_str(str_col(row, 6)).unwrap_or_default(),
        resource_limits_json: serde_json::from_str(str_col(row, 7))
            .unwrap_or_else(|_| serde_json::json!({})),
        created_at: str_col(row, 8).to_string(),
        updated_at: str_col(row, 9).to_string(),
    }
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
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
        let path = format!("/tmp/test-sandbox-profiles-{}.db", uuid::Uuid::new_v4());
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn sandbox_profile_roundtrips() {
        let db = test_db();
        let profile =
            SandboxProfileRecord::new("sandbox-profile-1", "docker", "2026-05-08T12:00:00Z")
                .with_display_name("Docker strict")
                .with_policy(Some("disabled".into()), Some("rw".into()))
                .with_writable_path("/work")
                .with_env_allow("ARCHON_LOG")
                .with_resource_limits(serde_json::json!({"memory_mb": 1024}));

        insert_sandbox_profile(&db, &profile).unwrap();
        let restored = get_sandbox_profile(&db, "sandbox-profile-1")
            .unwrap()
            .unwrap();
        let listed = list_sandbox_profiles_by_backend(&db, "docker").unwrap();

        assert_eq!(restored.display_name.as_deref(), Some("Docker strict"));
        assert_eq!(restored.writable_paths, vec!["/work"]);
        assert_eq!(restored.env_allowlist, vec!["ARCHON_LOG"]);
        assert_eq!(restored.resource_limits_json["memory_mb"], 1024);
        assert_eq!(listed.len(), 1);
    }
}
