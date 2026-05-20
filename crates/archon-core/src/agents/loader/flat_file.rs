use std::path::Path;

use tracing::warn;

use super::AgentLoadError;
use crate::agents::definition::{AgentMeta, AgentSource, CustomAgentDefinition, PermissionMode};

fn extract_yaml_frontmatter_and_body(content: &str) -> Option<(String, String)> {
    let mut lines = content.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let mut frontmatter = String::new();
    for line in lines.by_ref() {
        if line.trim() == "---" {
            let body = lines.collect::<Vec<_>>().join("\n");
            return Some((frontmatter, body));
        }
        frontmatter.push_str(line);
        frontmatter.push('\n');
    }
    None
}

/// Parse the `tools` YAML field, which can be either a comma-separated
/// string (`"Read, Grep, Glob"`) or a YAML array (`["Read", "Grep"]`).
fn parse_tools_field(value: &serde_json::Value) -> Option<Vec<String>> {
    match value {
        serde_json::Value::Array(arr) => {
            let tools: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
            if tools.is_empty() { None } else { Some(tools) }
        }
        serde_json::Value::String(s) => {
            let tools: Vec<String> = s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect();
            if tools.is_empty() { None } else { Some(tools) }
        }
        _ => None,
    }
}

/// Parse a single flat-file `.md` agent with YAML frontmatter into a
/// `CustomAgentDefinition`. Returns `Ok(None)` for files without
/// frontmatter (READMEs etc. — silently skipped).
fn parse_flat_file_agent(
    path: &Path,
    source: &AgentSource,
) -> Result<Option<CustomAgentDefinition>, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read error: {e}"))?;

    let (frontmatter, body) = match extract_yaml_frontmatter_and_body(&content) {
        Some(fm) => fm,
        None => return Ok(None),
    };

    let parsed: serde_json::Value =
        serde_yaml_ng::from_str(&frontmatter).map_err(|e| format!("YAML parse error: {e}"))?;

    let filename_stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let agent_type = parsed
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .unwrap_or_else(|| filename_stem.to_string());

    let description = parsed
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_default();

    let allowed_tools = parsed.get("tools").and_then(parse_tools_field);

    let model = parsed
        .get("model")
        .and_then(|v| v.as_str())
        .map(String::from);

    let effort = parsed
        .get("effort")
        .and_then(|v| v.as_str())
        .map(String::from);

    let permission_mode = parsed
        .pointer("/permissions/default_mode")
        .or_else(|| parsed.get("permission_mode"))
        .and_then(|v| v.as_str())
        .and_then(PermissionMode::from_str_opt);

    let color = parsed
        .get("color")
        .and_then(|v| v.as_str())
        .map(String::from);

    let tags = parsed
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let base_dir = path.parent().map(|p| p.to_string_lossy().into_owned());
    let mut meta = AgentMeta::default();
    if let Some(version) = parsed.get("version").and_then(|v| v.as_str()) {
        meta.version = version.to_string();
    }

    Ok(Some(CustomAgentDefinition {
        agent_type,
        system_prompt: body.trim().to_string(),
        description,
        allowed_tools,
        disallowed_tools: None,
        tool_guidance: String::new(),
        model,
        effort,
        max_turns: None,
        permission_mode,
        background: false,
        initial_prompt: None,
        color,
        memory_scope: None,
        recall_queries: Vec::new(),
        leann_queries: Vec::new(),
        tags,
        source: source.clone(),
        meta,
        filename: Some(filename_stem.to_string()),
        base_dir,
        isolation: None,
        mcp_servers: None,
        required_mcp_servers: None,
        hooks: None,
        skills: None,
        omit_claude_md: false,
        critical_system_reminder: None,
    }))
}

/// Load all flat-file YAML-frontmatter agents from a root directory,
/// recursively walking all subdirectories except `custom/` (handled by
/// `load_custom_agents`) and hidden/private dirs (`.` or `_` prefix).
///
/// Each `.md` file with a valid YAML frontmatter becomes one
/// `CustomAgentDefinition`. Files without frontmatter are silently
/// skipped (READMEs etc.). Malformed YAML is logged at `warn!` level
/// and the file is skipped — the walk continues.
pub fn load_flat_file_agents(
    agents_root: &Path,
    source: AgentSource,
) -> Result<Vec<CustomAgentDefinition>, AgentLoadError> {
    if !agents_root.exists() {
        return Ok(Vec::new());
    }

    let mut agents = Vec::new();

    for entry in walkdir::WalkDir::new(agents_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            e.file_name()
                .to_str()
                .map(|s| !s.starts_with('.') && !s.starts_with('_') && s != "custom")
                .unwrap_or(false)
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "walkdir error in flat-file agent discovery");
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        match parse_flat_file_agent(path, &source) {
            Ok(Some(agent)) => agents.push(agent),
            Ok(None) => {} // no frontmatter — silent skip
            Err(e) => {
                warn!(path = %path.display(), error = %e, "failed to parse flat-file agent");
            }
        }
    }

    Ok(agents)
}
