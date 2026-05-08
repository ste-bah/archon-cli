use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentProfileVersionRecord {
    pub version_id: String,
    pub agent_type: String,
    pub version_number: i64,
    pub parent_version_id: Option<String>,
    pub source: String,
    pub created_by_proposal_id: Option<String>,
    pub profile_json: serde_json::Value,
    pub prompt_hash: Option<String>,
    pub tools_hash: Option<String>,
    pub model_hash: Option<String>,
    pub memory_hash: Option<String>,
    pub is_active: bool,
    pub is_rollback_target: bool,
    pub created_at: String,
}

impl AgentProfileVersionRecord {
    pub fn new(
        version_id: impl Into<String>,
        agent_type: impl Into<String>,
        version_number: i64,
        source: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            version_id: version_id.into(),
            agent_type: agent_type.into(),
            version_number,
            parent_version_id: None,
            source: source.into(),
            created_by_proposal_id: None,
            profile_json: serde_json::json!({}),
            prompt_hash: None,
            tools_hash: None,
            model_hash: None,
            memory_hash: None,
            is_active: false,
            is_rollback_target: false,
            created_at: created_at.into(),
        }
    }

    pub fn with_parent(mut self, parent_version_id: impl Into<String>) -> Self {
        self.parent_version_id = Some(parent_version_id.into());
        self
    }

    pub fn with_proposal(mut self, proposal_id: impl Into<String>) -> Self {
        self.created_by_proposal_id = Some(proposal_id.into());
        self
    }

    pub fn with_profile_json(mut self, profile_json: serde_json::Value) -> Self {
        self.profile_json = profile_json;
        self
    }

    pub fn with_hashes(
        mut self,
        prompt_hash: Option<String>,
        tools_hash: Option<String>,
        model_hash: Option<String>,
        memory_hash: Option<String>,
    ) -> Self {
        self.prompt_hash = prompt_hash;
        self.tools_hash = tools_hash;
        self.model_hash = model_hash;
        self.memory_hash = memory_hash;
        self
    }

    pub fn mark_active(mut self) -> Self {
        self.is_active = true;
        self
    }

    pub fn mark_rollback_target(mut self) -> Self {
        self.is_rollback_target = true;
        self
    }
}

pub fn insert_agent_profile_version(
    db: &DbInstance,
    version: &AgentProfileVersionRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("vid".into(), DataValue::from(version.version_id.as_str()));
    params.insert("agent".into(), DataValue::from(version.agent_type.as_str()));
    params.insert("num".into(), DataValue::from(version.version_number));
    params.insert(
        "parent".into(),
        DataValue::from(version.parent_version_id.as_deref().unwrap_or("")),
    );
    params.insert("source".into(), DataValue::from(version.source.as_str()));
    params.insert(
        "proposal".into(),
        DataValue::from(version.created_by_proposal_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "profile".into(),
        DataValue::from(version.profile_json.to_string().as_str()),
    );
    params.insert(
        "prompt".into(),
        DataValue::from(version.prompt_hash.as_deref().unwrap_or("")),
    );
    params.insert(
        "tools".into(),
        DataValue::from(version.tools_hash.as_deref().unwrap_or("")),
    );
    params.insert(
        "model".into(),
        DataValue::from(version.model_hash.as_deref().unwrap_or("")),
    );
    params.insert(
        "memory".into(),
        DataValue::from(version.memory_hash.as_deref().unwrap_or("")),
    );
    params.insert("active".into(), DataValue::from(version.is_active));
    params.insert(
        "rollback".into(),
        DataValue::from(version.is_rollback_target),
    );
    params.insert(
        "created".into(),
        DataValue::from(version.created_at.as_str()),
    );

    db.run_script(
        profile_version_put_script(),
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| anyhow::anyhow!("insert agent_profile_versions failed: {e}"))?;
    Ok(())
}

pub fn get_agent_profile_version(
    db: &DbInstance,
    version_id: &str,
) -> Result<Option<AgentProfileVersionRecord>> {
    let mut params = BTreeMap::new();
    params.insert("vid".into(), DataValue::from(version_id));
    let result = db
        .run_script(
            profile_version_query("version_id = $vid"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get agent_profile_version failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_profile_version(row)))
}

pub fn list_agent_profile_versions(
    db: &DbInstance,
    agent_type: &str,
) -> Result<Vec<AgentProfileVersionRecord>> {
    let mut params = BTreeMap::new();
    params.insert("agent".into(), DataValue::from(agent_type));
    let result = db
        .run_script(
            profile_version_query("agent_type = $agent"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("list agent_profile_versions failed: {e}"))?;
    let mut versions: Vec<_> = result
        .rows
        .iter()
        .map(|row| row_to_profile_version(row))
        .collect();
    versions.sort_by(|a, b| b.version_number.cmp(&a.version_number));
    Ok(versions)
}

pub fn get_active_agent_profile_version(
    db: &DbInstance,
    agent_type: &str,
) -> Result<Option<AgentProfileVersionRecord>> {
    let mut params = BTreeMap::new();
    params.insert("agent".into(), DataValue::from(agent_type));
    let result = db
        .run_script(
            profile_version_query("active_for_agent"),
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("get active agent_profile_version failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_profile_version(row)))
}

fn profile_version_put_script() -> &'static str {
    "?[version_id, agent_type, version_number, parent_version_id, source, \
     created_by_proposal_id, profile_json, prompt_hash, tools_hash, \
     model_hash, memory_hash, is_active, is_rollback_target, created_at] \
     <- [[$vid, $agent, $num, $parent, $source, $proposal, $profile, \
     $prompt, $tools, $model, $memory, $active, $rollback, $created]] \
     :put agent_profile_versions { version_id => agent_type, version_number, \
     parent_version_id, source, created_by_proposal_id, profile_json, \
     prompt_hash, tools_hash, model_hash, memory_hash, is_active, \
     is_rollback_target, created_at }"
}

fn profile_version_query(predicate: &'static str) -> &'static str {
    match predicate {
        "version_id = $vid" => {
            "?[version_id, agent_type, version_number, parent_version_id, source, \
             created_by_proposal_id, profile_json, prompt_hash, tools_hash, \
             model_hash, memory_hash, is_active, is_rollback_target, created_at] \
             := *agent_profile_versions{version_id, agent_type, version_number, \
             parent_version_id, source, created_by_proposal_id, profile_json, \
             prompt_hash, tools_hash, model_hash, memory_hash, is_active, \
             is_rollback_target, created_at}, version_id = $vid"
        }
        "active_for_agent" => {
            "?[version_id, agent_type, version_number, parent_version_id, source, \
             created_by_proposal_id, profile_json, prompt_hash, tools_hash, \
             model_hash, memory_hash, is_active, is_rollback_target, created_at] \
             := *agent_profile_versions{version_id, agent_type, version_number, \
             parent_version_id, source, created_by_proposal_id, profile_json, \
             prompt_hash, tools_hash, model_hash, memory_hash, is_active, \
             is_rollback_target, created_at}, agent_type = $agent, is_active = true"
        }
        _ => {
            "?[version_id, agent_type, version_number, parent_version_id, source, \
             created_by_proposal_id, profile_json, prompt_hash, tools_hash, \
             model_hash, memory_hash, is_active, is_rollback_target, created_at] \
             := *agent_profile_versions{version_id, agent_type, version_number, \
             parent_version_id, source, created_by_proposal_id, profile_json, \
             prompt_hash, tools_hash, model_hash, memory_hash, is_active, \
             is_rollback_target, created_at}, agent_type = $agent"
        }
    }
}

fn row_to_profile_version(row: &[DataValue]) -> AgentProfileVersionRecord {
    AgentProfileVersionRecord {
        version_id: str_col(row, 0).to_string(),
        agent_type: str_col(row, 1).to_string(),
        version_number: row[2].get_int().unwrap_or(1),
        parent_version_id: non_empty(str_col(row, 3)),
        source: str_col(row, 4).to_string(),
        created_by_proposal_id: non_empty(str_col(row, 5)),
        profile_json: serde_json::from_str(str_col(row, 6))
            .unwrap_or_else(|_| serde_json::json!({})),
        prompt_hash: non_empty(str_col(row, 7)),
        tools_hash: non_empty(str_col(row, 8)),
        model_hash: non_empty(str_col(row, 9)),
        memory_hash: non_empty(str_col(row, 10)),
        is_active: row[11].get_bool().unwrap_or(false),
        is_rollback_target: row[12].get_bool().unwrap_or(false),
        created_at: str_col(row, 13).to_string(),
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
        let path = format!(
            "/tmp/test-agent-profile-versions-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = DbInstance::new("sqlite", &path, "").unwrap();
        crate::schema::ensure_learning_schema(&db).unwrap();
        db
    }

    #[test]
    fn agent_profile_version_roundtrips() {
        let db = test_db();
        let version = AgentProfileVersionRecord::new(
            "agent-profile-1",
            "reviewer",
            1,
            "file_definition",
            "2026-05-08T12:00:00Z",
        )
        .with_profile_json(serde_json::json!({"model": "claude-sonnet-4-6"}))
        .with_hashes(
            Some("sha256:prompt".to_string()),
            Some("sha256:tools".to_string()),
            None,
            None,
        )
        .mark_active()
        .mark_rollback_target();

        insert_agent_profile_version(&db, &version).unwrap();
        let restored = get_agent_profile_version(&db, "agent-profile-1")
            .unwrap()
            .unwrap();

        assert_eq!(restored.agent_type, "reviewer");
        assert_eq!(restored.profile_json["model"], "claude-sonnet-4-6");
        assert_eq!(restored.prompt_hash.as_deref(), Some("sha256:prompt"));
        assert!(restored.is_active);
        assert!(restored.is_rollback_target);
    }

    #[test]
    fn profile_versions_list_and_active_lookup() {
        let db = test_db();
        insert_agent_profile_version(
            &db,
            &AgentProfileVersionRecord::new(
                "agent-profile-1",
                "planner",
                1,
                "file_definition",
                "2026-05-08T12:00:00Z",
            ),
        )
        .unwrap();
        insert_agent_profile_version(
            &db,
            &AgentProfileVersionRecord::new(
                "agent-profile-2",
                "planner",
                2,
                "governed_proposal",
                "2026-05-08T12:01:00Z",
            )
            .with_parent("agent-profile-1")
            .with_proposal("agent-evo-prop-1")
            .mark_active(),
        )
        .unwrap();

        let versions = list_agent_profile_versions(&db, "planner").unwrap();
        let active = get_active_agent_profile_version(&db, "planner")
            .unwrap()
            .unwrap();

        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version_id, "agent-profile-2");
        assert_eq!(active.version_id, "agent-profile-2");
        assert_eq!(active.parent_version_id.as_deref(), Some("agent-profile-1"));
    }
}
