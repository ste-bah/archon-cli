//! Governed agent profile overlays.
//!
//! This module is deliberately pure: Cozo owns durable profile versions in
//! `archon-learning`, while core owns how an already-approved profile JSON can
//! affect an in-memory agent definition.

use std::collections::BTreeSet;

use chrono::Utc;
use serde_json::Value;

use crate::agents::definition::{CustomAgentDefinition, EvolutionEntry, PermissionMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProfileOverlayReport {
    pub version_id: String,
    pub applied_fields: Vec<String>,
    pub ignored_fields: Vec<String>,
}

impl AgentProfileOverlayReport {
    fn new(version_id: String) -> Self {
        Self {
            version_id,
            applied_fields: Vec::new(),
            ignored_fields: Vec::new(),
        }
    }

    fn applied(&mut self, field: &str) {
        push_unique(&mut self.applied_fields, field);
    }

    fn ignored(&mut self, field: &str) {
        push_unique(&mut self.ignored_fields, field);
    }

    pub fn applied_count(&self) -> usize {
        self.applied_fields.len()
    }
}

pub fn apply_agent_profile_overlay(
    agent: &mut CustomAgentDefinition,
    version_id: impl Into<String>,
    profile_json: &Value,
) -> AgentProfileOverlayReport {
    let version_id = version_id.into();
    let mut report = AgentProfileOverlayReport::new(version_id.clone());
    let Some(profile) = overlay_object(profile_json) else {
        report.ignored("<non_object_profile>");
        return report;
    };

    for key in sorted_keys(profile) {
        match key.as_str() {
            "system_prompt" => apply_string(profile, &key, &mut report, |value| {
                agent.system_prompt = value;
            }),
            "system_prompt_append" => apply_string(profile, &key, &mut report, |value| {
                append_block(&mut agent.system_prompt, &value);
            }),
            "description" => apply_string(profile, &key, &mut report, |value| {
                agent.description = value;
            }),
            "tool_guidance" => apply_string(profile, &key, &mut report, |value| {
                agent.tool_guidance = value;
            }),
            "tool_guidance_append" => apply_string(profile, &key, &mut report, |value| {
                append_block(&mut agent.tool_guidance, &value);
            }),
            "disallowed_tools" => apply_string_vec(profile, &key, &mut report, |value| {
                agent.disallowed_tools = Some(value);
            }),
            "model" => apply_string(profile, &key, &mut report, |value| {
                agent.model = Some(value);
            }),
            "effort" => apply_string(profile, &key, &mut report, |value| {
                agent.effort = Some(value);
            }),
            "max_turns" => apply_u32(profile, &key, &mut report, |value| {
                agent.max_turns = Some(value);
            }),
            "permission_mode" => apply_permission_mode(profile, &key, &mut report, agent),
            "background" => apply_bool(profile, &key, &mut report, |value| {
                agent.background = value;
            }),
            "initial_prompt" => apply_string(profile, &key, &mut report, |value| {
                agent.initial_prompt = Some(value);
            }),
            "color" => apply_string(profile, &key, &mut report, |value| {
                agent.color = Some(value);
            }),
            "recall_queries" => apply_string_vec(profile, &key, &mut report, |value| {
                agent.recall_queries = value;
            }),
            "leann_queries" => apply_string_vec(profile, &key, &mut report, |value| {
                agent.leann_queries = value;
            }),
            "tags" => apply_string_vec(profile, &key, &mut report, |value| {
                agent.tags = value;
            }),
            "isolation" => apply_string(profile, &key, &mut report, |value| {
                agent.isolation = Some(value);
            }),
            "omit_claude_md" => apply_bool(profile, &key, &mut report, |value| {
                agent.omit_claude_md = value;
            }),
            "critical_system_reminder" => apply_string(profile, &key, &mut report, |value| {
                agent.critical_system_reminder = Some(value);
            }),
            key if is_blocked_field(key) => report.ignored(key),
            key => report.ignored(key),
        }
    }

    if report.applied_count() > 0 {
        agent.meta.evolution_history.push(EvolutionEntry {
            version: version_id,
            timestamp: Utc::now(),
            change_type: "active_profile_overlay".to_string(),
            description: format!(
                "Applied governed profile overlay ({} field(s))",
                report.applied_count()
            ),
        });
    }
    report
}

fn overlay_object(profile_json: &Value) -> Option<&serde_json::Map<String, Value>> {
    profile_json
        .get("overrides")
        .and_then(Value::as_object)
        .or_else(|| profile_json.as_object())
}

fn sorted_keys(profile: &serde_json::Map<String, Value>) -> Vec<String> {
    profile
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn apply_string<F>(
    profile: &serde_json::Map<String, Value>,
    key: &str,
    report: &mut AgentProfileOverlayReport,
    apply: F,
) where
    F: FnOnce(String),
{
    match profile.get(key).and_then(Value::as_str).map(str::trim) {
        Some(value) if !value.is_empty() => {
            apply(value.to_string());
            report.applied(key);
        }
        _ => report.ignored(key),
    }
}

fn apply_string_vec<F>(
    profile: &serde_json::Map<String, Value>,
    key: &str,
    report: &mut AgentProfileOverlayReport,
    apply: F,
) where
    F: FnOnce(Vec<String>),
{
    let Some(values) = profile.get(key).and_then(string_vec) else {
        report.ignored(key);
        return;
    };
    apply(values);
    report.applied(key);
}

fn apply_u32<F>(
    profile: &serde_json::Map<String, Value>,
    key: &str,
    report: &mut AgentProfileOverlayReport,
    apply: F,
) where
    F: FnOnce(u32),
{
    match profile
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
    {
        Some(value) => {
            apply(value);
            report.applied(key);
        }
        None => report.ignored(key),
    }
}

fn apply_bool<F>(
    profile: &serde_json::Map<String, Value>,
    key: &str,
    report: &mut AgentProfileOverlayReport,
    apply: F,
) where
    F: FnOnce(bool),
{
    match profile.get(key).and_then(Value::as_bool) {
        Some(value) => {
            apply(value);
            report.applied(key);
        }
        None => report.ignored(key),
    }
}

fn apply_permission_mode(
    profile: &serde_json::Map<String, Value>,
    key: &str,
    report: &mut AgentProfileOverlayReport,
    agent: &mut CustomAgentDefinition,
) {
    let Some(value) = profile.get(key).and_then(Value::as_str) else {
        report.ignored(key);
        return;
    };
    if let Some(mode) = PermissionMode::from_str_opt(value) {
        agent.permission_mode = Some(mode);
        report.applied(key);
    } else {
        report.ignored(key);
    }
}

fn append_block(target: &mut String, value: &str) {
    if target.trim().is_empty() {
        *target = value.trim().to_string();
    } else {
        target.push_str("\n\n");
        target.push_str(value.trim());
    }
}

fn string_vec(value: &Value) -> Option<Vec<String>> {
    let values = value.as_array()?;
    let strings: Vec<_> = values
        .iter()
        .filter_map(|value| value.as_str().map(str::trim))
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect();
    (!strings.is_empty()).then_some(strings)
}

fn is_blocked_field(key: &str) -> bool {
    matches!(
        key,
        "allowed_tools"
            | "allow_host_shell_fallback"
            | "allow_provider_injection"
            | "anthropic"
            | "anthropic_spoof"
            | "claude_md"
            | "claude_memory"
            | "docker"
            | "identity"
            | "identity_mode"
            | "identity_spoof"
            | "memory_file_path"
            | "memory_files"
            | "mcp_servers"
            | "openai_codex"
            | "openshell"
            | "provider"
            | "provider_id"
            | "provider_injection"
            | "required_mcp_servers"
            | "sandbox_backend"
            | "sandbox_profile"
            | "skills"
            | "spoof"
            | "ssh"
            | "hooks"
    )
}

fn push_unique(values: &mut Vec<String>, value: &str) {
    if !values.iter().any(|existing| existing == value) {
        values.push(value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> CustomAgentDefinition {
        CustomAgentDefinition {
            agent_type: "reviewer".to_string(),
            system_prompt: "Review code.".to_string(),
            tool_guidance: "Use Read first.".to_string(),
            ..CustomAgentDefinition::default()
        }
    }

    #[test]
    fn overlay_applies_profile_fields_and_records_history() {
        let mut agent = agent();
        let report = apply_agent_profile_overlay(
            &mut agent,
            "agent-profile-2",
            &serde_json::json!({
                "overrides": {
                    "system_prompt_append": "Require citations.",
                    "disallowed_tools": ["Bash"],
                    "model": "claude-sonnet-4-6",
                    "permission_mode": "plan",
                    "max_turns": 6
                }
            }),
        );

        assert!(agent.system_prompt.contains("Require citations."));
        assert_eq!(agent.disallowed_tools, Some(vec!["Bash".into()]));
        assert_eq!(agent.model.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(agent.permission_mode, Some(PermissionMode::Plan));
        assert_eq!(agent.max_turns, Some(6));
        assert_eq!(agent.meta.evolution_history[0].version, "agent-profile-2");
        assert_eq!(report.applied_count(), 5);
    }

    #[test]
    fn overlay_ignores_provider_spoof_sandbox_and_memory_file_fields() {
        let mut agent = agent();
        let report = apply_agent_profile_overlay(
            &mut agent,
            "agent-profile-3",
            &serde_json::json!({
                "provider": "openai-codex",
                "identity_spoof": false,
                "allow_provider_injection": true,
                "openshell": {"providers": ["anthropic"]},
                "sandbox_backend": "disabled",
                "memory_files": [".claude/memory.md"],
                "system_prompt_append": "Safe prompt change."
            }),
        );

        assert_eq!(agent.system_prompt, "Review code.\n\nSafe prompt change.");
        assert_eq!(report.applied_fields, vec!["system_prompt_append"]);
        assert!(report.ignored_fields.contains(&"provider".to_string()));
        assert!(
            report
                .ignored_fields
                .contains(&"identity_spoof".to_string())
        );
        assert!(
            report
                .ignored_fields
                .contains(&"allow_provider_injection".to_string())
        );
        assert!(report.ignored_fields.contains(&"openshell".to_string()));
        assert!(
            report
                .ignored_fields
                .contains(&"sandbox_backend".to_string())
        );
        assert!(report.ignored_fields.contains(&"memory_files".to_string()));
    }

    #[test]
    fn overlay_ignores_tool_mcp_skill_and_hook_grants() {
        let mut agent = agent();
        let report = apply_agent_profile_overlay(
            &mut agent,
            "agent-profile-5",
            &serde_json::json!({
                "overrides": {
                    "allowed_tools": ["Bash"],
                    "mcp_servers": ["filesystem"],
                    "required_mcp_servers": ["filesystem"],
                    "skills": ["deploy"],
                    "hooks": {"PermissionRequest": [{"matcher": "Bash"}]},
                    "tool_guidance_append": "Use safer alternatives first."
                }
            }),
        );

        assert!(agent.allowed_tools.is_none());
        assert!(agent.mcp_servers.is_none());
        assert!(agent.required_mcp_servers.is_none());
        assert!(agent.skills.is_none());
        assert!(agent.hooks.is_none());
        assert_eq!(report.applied_fields, vec!["tool_guidance_append"]);
        assert!(report.ignored_fields.contains(&"allowed_tools".to_string()));
        assert!(report.ignored_fields.contains(&"mcp_servers".to_string()));
        assert!(report.ignored_fields.contains(&"skills".to_string()));
        assert!(report.ignored_fields.contains(&"hooks".to_string()));
    }

    #[test]
    fn non_object_profile_is_ignored() {
        let mut agent = agent();
        let report =
            apply_agent_profile_overlay(&mut agent, "agent-profile-4", &serde_json::json!(true));

        assert!(report.applied_fields.is_empty());
        assert_eq!(report.ignored_fields, vec!["<non_object_profile>"]);
        assert!(agent.meta.evolution_history.is_empty());
    }
}
