use std::collections::BTreeSet;

use anyhow::Result;
use archon_core::agents::evolution::ToolAccessProfileDiff;
use archon_learning::agent_profile_versions::AgentProfileVersionRecord;
use cozo::DbInstance;

pub(super) fn render_permission_profile_diff_from_db(
    db: &DbInstance,
    agent_type: &str,
    from_version_id: &str,
    to_version_id: &str,
    json: bool,
) -> Result<String> {
    let from = load_agent_profile_version(db, agent_type, from_version_id)?;
    let to = load_agent_profile_version(db, agent_type, to_version_id)?;
    let diff = profile_permission_diff(&from, &to);
    if json {
        return Ok(format!("{}\n", serde_json::to_string_pretty(&diff)?));
    }

    let mut out = String::from("Permission profile diff (Cozo)\n\n");
    out.push_str(&format!("Agent: {}\n", agent_type));
    out.push_str(&format!("From:  {}\n", from.version_id));
    out.push_str(&format!("To:    {}\n", to.version_id));
    out.push_str(&format!("Risk:  {:?}\n", diff.risk_level()));
    out.push_str(&format!(
        "Expands permissions: {}\n",
        yes_no(diff.expands_permissions())
    ));
    out.push_str(&format!(
        "Added tools: {}\n",
        list_or_none(&diff.added_tools)
    ));
    out.push_str(&format!(
        "Removed tools: {}\n",
        list_or_none(&diff.removed_tools)
    ));
    out.push_str(&format!(
        "Added MCP servers: {}\n",
        list_or_none(&diff.added_mcp_servers)
    ));
    out.push_str(&format!(
        "Added skills: {}\n",
        list_or_none(&diff.added_skills)
    ));
    out.push_str(&format!(
        "Permission mode: {} -> {}\n",
        mode_label(diff.permission_mode_from.as_ref()),
        mode_label(diff.permission_mode_to.as_ref())
    ));
    out.push_str(&format!(
        "Sandbox backend: {} -> {}\n",
        diff.sandbox_backend_from.as_deref().unwrap_or("-"),
        diff.sandbox_backend_to.as_deref().unwrap_or("-")
    ));
    out.push_str("Guardrail: this command is read-only and never applies grants.\n");
    Ok(out)
}

fn load_agent_profile_version(
    db: &DbInstance,
    agent_type: &str,
    version_id: &str,
) -> Result<AgentProfileVersionRecord> {
    let version =
        archon_learning::agent_profile_versions::get_agent_profile_version(db, version_id)?
            .ok_or_else(|| anyhow::anyhow!("profile version not found: {version_id}"))?;
    if version.agent_type != agent_type {
        anyhow::bail!(
            "profile version {} belongs to agent {}, not {}",
            version_id,
            version.agent_type,
            agent_type
        );
    }
    Ok(version)
}

fn profile_permission_diff(
    from: &AgentProfileVersionRecord,
    to: &AgentProfileVersionRecord,
) -> ToolAccessProfileDiff {
    let from = PermissionProfileView::from_profile_json(&from.profile_json);
    let to = PermissionProfileView::from_profile_json(&to.profile_json);
    ToolAccessProfileDiff {
        added_tools: added(&from.allowed_tools, &to.allowed_tools),
        removed_tools: removed(&from.allowed_tools, &to.allowed_tools),
        added_mcp_servers: added(&from.mcp_servers, &to.mcp_servers),
        removed_mcp_servers: removed(&from.mcp_servers, &to.mcp_servers),
        added_skills: added(&from.skills, &to.skills),
        removed_skills: removed(&from.skills, &to.skills),
        hooks_added: !to.hooks.is_empty() && from.hooks.is_empty(),
        permission_mode_from: from.permission_mode,
        permission_mode_to: to.permission_mode,
        sandbox_backend_from: from.sandbox_backend,
        sandbox_backend_to: to.sandbox_backend,
    }
}

#[derive(Default)]
struct PermissionProfileView {
    allowed_tools: Vec<String>,
    mcp_servers: Vec<String>,
    skills: Vec<String>,
    hooks: Vec<String>,
    permission_mode: Option<archon_core::agents::definition::PermissionMode>,
    sandbox_backend: Option<String>,
}

impl PermissionProfileView {
    fn from_profile_json(profile_json: &serde_json::Value) -> Self {
        let overrides = profile_json.get("overrides").unwrap_or(profile_json);
        Self {
            allowed_tools: string_array(overrides, "allowed_tools"),
            mcp_servers: string_array(overrides, "mcp_servers"),
            skills: string_array(overrides, "skills"),
            hooks: string_array(overrides, "hooks"),
            permission_mode: string_field(overrides, "permission_mode").and_then(|value| {
                archon_core::agents::definition::PermissionMode::from_str_opt(&value)
            }),
            sandbox_backend: string_field(overrides, "sandbox_backend"),
        }
    }
}

fn string_array(value: &serde_json::Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn added(from: &[String], to: &[String]) -> Vec<String> {
    set_difference(to, from)
}

fn removed(from: &[String], to: &[String]) -> Vec<String> {
    set_difference(from, to)
}

fn set_difference(left: &[String], right: &[String]) -> Vec<String> {
    let right = right.iter().collect::<BTreeSet<_>>();
    left.iter()
        .filter(|value| !right.contains(value))
        .cloned()
        .collect()
}

fn list_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn mode_label(mode: Option<&archon_core::agents::definition::PermissionMode>) -> &'static str {
    mode.map(|mode| mode.as_str()).unwrap_or("-")
}
