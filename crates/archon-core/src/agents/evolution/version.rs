use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProfileVersionSource {
    FileDefinition,
    GovernedProposal,
    ManualOperator,
    Rollback,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentProfileVersion {
    pub version_id: String,
    pub agent_type: String,
    pub version_number: i64,
    pub parent_version_id: Option<String>,
    pub source: AgentProfileVersionSource,
    pub created_by_proposal_id: Option<String>,
    pub profile_json: serde_json::Value,
    pub prompt_hash: Option<String>,
    pub tools_hash: Option<String>,
    pub model_hash: Option<String>,
    pub memory_hash: Option<String>,
    pub is_active: bool,
    pub is_rollback_target: bool,
    pub created_at: DateTime<Utc>,
}

impl AgentProfileVersion {
    pub fn new(
        agent_type: impl Into<String>,
        version_number: i64,
        source: AgentProfileVersionSource,
        profile_json: serde_json::Value,
    ) -> Self {
        let mut version = Self {
            version_id: agent_profile_version_id(),
            agent_type: agent_type.into(),
            version_number,
            parent_version_id: None,
            source,
            created_by_proposal_id: None,
            profile_json,
            prompt_hash: None,
            tools_hash: None,
            model_hash: None,
            memory_hash: None,
            is_active: false,
            is_rollback_target: false,
            created_at: Utc::now(),
        };
        version.recompute_hashes();
        version
    }

    pub fn with_parent(mut self, parent_version_id: impl Into<String>) -> Self {
        self.parent_version_id = Some(parent_version_id.into());
        self
    }

    pub fn with_proposal(mut self, proposal_id: impl Into<String>) -> Self {
        self.created_by_proposal_id = Some(proposal_id.into());
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

    pub fn recompute_hashes(&mut self) {
        self.prompt_hash = hash_profile_fields(
            &self.profile_json,
            &[
                "system_prompt",
                "initial_prompt",
                "critical_system_reminder",
                "tool_guidance",
            ],
        );
        self.tools_hash = hash_profile_fields(
            &self.profile_json,
            &[
                "allowed_tools",
                "disallowed_tools",
                "permission_mode",
                "mcp_servers",
                "required_mcp_servers",
                "skills",
                "hooks",
            ],
        );
        self.model_hash =
            hash_profile_fields(&self.profile_json, &["model", "effort", "max_turns"]);
        self.memory_hash = hash_profile_fields(
            &self.profile_json,
            &["memory_scope", "recall_queries", "leann_queries"],
        );
    }

    pub fn effective_profile_hash(&self) -> String {
        stable_hash(&self.profile_json)
    }
}

pub fn agent_profile_version_id() -> String {
    format!("agent-profile-{}", uuid::Uuid::new_v4())
}

fn hash_profile_fields(profile_json: &serde_json::Value, fields: &[&str]) -> Option<String> {
    let object = profile_json.as_object()?;
    let mut section = serde_json::Map::new();

    for field in fields {
        if let Some(value) = object.get(*field) {
            section.insert((*field).to_string(), value.clone());
        }
    }

    if section.is_empty() {
        None
    } else {
        Some(stable_hash(&serde_json::Value::Object(section)))
    }
}

fn stable_hash(value: &serde_json::Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_json() -> serde_json::Value {
        serde_json::json!({
            "system_prompt": "Review the code carefully.",
            "allowed_tools": ["Read"],
            "disallowed_tools": ["Bash"],
            "model": "claude-sonnet-4-6",
            "effort": "high",
            "memory_scope": "project",
            "recall_queries": ["recent code review corrections"]
        })
    }

    #[test]
    fn profile_version_serializes_core_prd_fields() {
        let version = AgentProfileVersion::new(
            "reviewer",
            7,
            AgentProfileVersionSource::GovernedProposal,
            profile_json(),
        )
        .with_parent("agent-profile-6")
        .with_proposal("agent-evo-prop-1")
        .mark_active();

        let value = serde_json::to_value(&version).unwrap();

        assert_eq!(value["agent_type"], "reviewer");
        assert_eq!(value["version_number"], 7);
        assert_eq!(value["source"], "governed_proposal");
        assert_eq!(
            version.parent_version_id.as_deref(),
            Some("agent-profile-6")
        );
        assert!(version.version_id.starts_with("agent-profile-"));
        assert!(version.is_active);
    }

    #[test]
    fn profile_version_hashes_independent_sections() {
        let version = AgentProfileVersion::new(
            "reviewer",
            1,
            AgentProfileVersionSource::FileDefinition,
            profile_json(),
        );

        assert!(
            version
                .prompt_hash
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            version
                .tools_hash
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            version
                .model_hash
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(
            version
                .memory_hash
                .as_deref()
                .unwrap()
                .starts_with("sha256:")
        );
        assert!(version.effective_profile_hash().starts_with("sha256:"));
    }

    #[test]
    fn absent_sections_have_no_section_hash() {
        let version = AgentProfileVersion::new(
            "minimal",
            1,
            AgentProfileVersionSource::FileDefinition,
            serde_json::json!({"description": "only description"}),
        );

        assert_eq!(version.prompt_hash, None);
        assert_eq!(version.tools_hash, None);
        assert_eq!(version.model_hash, None);
        assert_eq!(version.memory_hash, None);
    }
}
