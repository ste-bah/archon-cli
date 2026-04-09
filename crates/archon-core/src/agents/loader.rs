use std::path::Path;

use tracing::warn;

use super::definition::{
    AgentMemoryScope, AgentMeta, AgentQuality, AgentSource, CustomAgentDefinition, PermissionMode,
};
use crate::hooks::{HookConfig, HookEvent};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AgentLoadError {
    #[error("failed to read directory {path}: {source}")]
    ReadDir {
        path: String,
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load all custom agents from `dir` (e.g. `.archon/agents/custom/`).
///
/// Each immediate subdirectory (not starting with `_`) is treated as an agent
/// in the 6-file format. Missing or malformed files produce warnings, not errors.
pub fn load_custom_agents(
    dir: &Path,
    source: AgentSource,
) -> Result<Vec<CustomAgentDefinition>, AgentLoadError> {
    let entries = std::fs::read_dir(dir).map_err(|e| AgentLoadError::ReadDir {
        path: dir.display().to_string(),
        source: e,
    })?;

    let mut agents = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip template/schema/archived directories
        if name.starts_with('_') {
            continue;
        }

        agents.push(load_single_agent(&path, &name, source.clone()));
    }

    Ok(agents)
}

// ---------------------------------------------------------------------------
// Single agent loader
// ---------------------------------------------------------------------------

fn load_single_agent(dir: &Path, name: &str, source: AgentSource) -> CustomAgentDefinition {
    let agent_md = read_file_or_default(dir, "agent.md", name);
    let behavior_md = read_file_or_default(dir, "behavior.md", name);
    let context_md = read_file_or_default(dir, "context.md", name);
    let tools_md = read_file_or_default(dir, "tools.md", name);
    let memory_keys_str = read_file_or_default(dir, "memory-keys.json", name);
    let meta_str = read_file_or_default(dir, "meta.json", name);

    let description = extract_description(&agent_md);
    let system_prompt = assemble_system_prompt(&agent_md, &behavior_md, &context_md);
    let allowed_tools = extract_tools(&tools_md);
    let tool_guidance = extract_tool_guidance(&tools_md);
    let (recall_queries, leann_queries, tags, memory_scope) =
        parse_memory_keys(&memory_keys_str, name);
    let (meta, exec_config) = parse_meta_json(&meta_str, name);

    CustomAgentDefinition {
        agent_type: name.to_string(),
        system_prompt,
        description,
        allowed_tools,
        disallowed_tools: exec_config.disallowed_tools,
        tool_guidance,
        model: exec_config.model,
        effort: exec_config.effort,
        max_turns: exec_config.max_turns,
        permission_mode: exec_config.permission_mode,
        background: exec_config.background,
        initial_prompt: exec_config.initial_prompt,
        color: exec_config.color,
        memory_scope,
        recall_queries,
        leann_queries,
        tags,
        source,
        meta,
        filename: Some(name.to_string()),
        base_dir: Some(dir.to_string_lossy().into_owned()),
        isolation: exec_config.isolation,
        mcp_servers: exec_config.mcp_servers,
        required_mcp_servers: exec_config.required_mcp_servers,
        hooks: exec_config.hooks,
        skills: exec_config.skills,
        omit_claude_md: exec_config.omit_claude_md,
        critical_system_reminder: exec_config.critical_system_reminder,
    }
}

// ---------------------------------------------------------------------------
// File reading
// ---------------------------------------------------------------------------

fn read_file_or_default(dir: &Path, filename: &str, agent_name: &str) -> String {
    let path = dir.join(filename);
    match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => {
            warn!(
                agent = agent_name,
                file = filename,
                "missing agent file, using default"
            );
            String::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Description extraction
// ---------------------------------------------------------------------------

/// Extract description from agent.md:
/// 1. Find `## INTENT` section, take first non-empty paragraph
/// 2. Fallback: first line stripped of `#` prefix
pub fn extract_description(agent_md: &str) -> String {
    let lines: Vec<&str> = agent_md.lines().collect();

    // Look for ## INTENT section
    let intent_start = lines.iter().position(|l| {
        let trimmed = l.trim().to_uppercase();
        trimmed == "## INTENT" || trimmed.starts_with("## INTENT")
    });

    if let Some(start) = intent_start {
        let content_lines = &lines[start + 1..];
        let mut paragraph = String::new();

        for line in content_lines {
            // Stop at next heading
            if line.trim_start().starts_with("##") {
                break;
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if !paragraph.is_empty() {
                    break; // End of first paragraph
                }
                continue;
            }
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(trimmed);
        }

        if !paragraph.is_empty() {
            return paragraph;
        }
    }

    // Fallback: first line stripped of '#'
    agent_md
        .lines()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.trim().trim_start_matches('#').trim().to_string())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// System prompt assembly + truncation
// ---------------------------------------------------------------------------

/// Maximum agent prompt budget in tokens. 1 token ~= 4 chars (conservative).
const MAX_AGENT_PROMPT_TOKENS: usize = 8192;
const CHARS_PER_TOKEN: usize = 4;

fn assemble_system_prompt(agent_md: &str, behavior_md: &str, context_md: &str) -> String {
    truncate_agent_prompt(
        agent_md.trim(),
        behavior_md.trim(),
        context_md.trim(),
        MAX_AGENT_PROMPT_TOKENS,
    )
}

/// Find the largest byte index <= `idx` that is a valid char boundary.
fn floor_char_boundary(s: &str, idx: usize) -> usize {
    if idx >= s.len() {
        return s.len();
    }
    let mut i = idx;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Assemble and truncate agent prompt with priority cascade:
/// 1. context_md trimmed first (least critical)
/// 2. behavior_md trimmed second
/// 3. agent_md NEVER trimmed (core identity)
pub fn truncate_agent_prompt(
    agent_md: &str,
    behavior_md: &str,
    context_md: &str,
    max_tokens: usize,
) -> String {
    let max_chars = max_tokens * CHARS_PER_TOKEN;
    // Account for "\n\n" separators (up to 4 chars for 2 separators)
    let separator_chars = 4;
    let total = agent_md.len() + behavior_md.len() + context_md.len() + separator_chars;

    if total <= max_chars {
        let mut parts = Vec::new();
        if !agent_md.is_empty() {
            parts.push(agent_md);
        }
        if !behavior_md.is_empty() {
            parts.push(behavior_md);
        }
        if !context_md.is_empty() {
            parts.push(context_md);
        }
        return parts.join("\n\n");
    }

    // Budget remaining after agent_md (never trimmed) + separators
    let remaining = max_chars.saturating_sub(agent_md.len() + separator_chars);

    // Try trimming context_md first
    let context_budget = remaining.saturating_sub(behavior_md.len());
    let trimmed_context = if context_md.len() > context_budget {
        warn!(
            trimmed_section = "context.md",
            original_len = context_md.len(),
            budget = context_budget,
            "truncating agent prompt"
        );
        if context_budget > 20 {
            let end = floor_char_boundary(context_md, context_budget.saturating_sub(16));
            format!("{}... [truncated]", &context_md[..end])
        } else {
            String::new()
        }
    } else {
        context_md.to_string()
    };

    // If still over, trim behavior_md
    let behavior_budget = remaining.saturating_sub(trimmed_context.len());
    let trimmed_behavior = if behavior_md.len() > behavior_budget {
        warn!(
            trimmed_section = "behavior.md",
            original_len = behavior_md.len(),
            budget = behavior_budget,
            "truncating agent prompt"
        );
        if behavior_budget > 20 {
            let end = floor_char_boundary(behavior_md, behavior_budget.saturating_sub(16));
            format!("{}... [truncated]", &behavior_md[..end])
        } else {
            String::new()
        }
    } else {
        behavior_md.to_string()
    };

    let mut parts = Vec::new();
    if !agent_md.is_empty() {
        parts.push(agent_md.to_string());
    }
    if !trimmed_behavior.is_empty() {
        parts.push(trimmed_behavior);
    }
    if !trimmed_context.is_empty() {
        parts.push(trimmed_context);
    }
    parts.join("\n\n")
}

// ---------------------------------------------------------------------------
// Tool extraction
// ---------------------------------------------------------------------------

/// Extract tools from `## Primary Tools` section in tools.md.
/// Returns None if no tools found (meaning all tools allowed).
pub fn extract_tools(tools_md: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = tools_md.lines().collect();

    let section_start = lines.iter().position(|l| {
        let trimmed = l.trim().to_uppercase();
        trimmed.starts_with("## PRIMARY TOOLS")
    });

    let start = match section_start {
        Some(idx) => idx + 1,
        None => return None,
    };

    let mut tools = Vec::new();
    for line in &lines[start..] {
        let trimmed = line.trim();
        // Stop at next heading
        if trimmed.starts_with("##") {
            break;
        }
        // Parse bullet items: "- **Glob**: description" or "- Glob"
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let name = rest
                .trim_start_matches("**")
                .split(|c: char| c == '*' || c == ':' || c == ' ')
                .next()
                .unwrap_or("")
                .trim();
            if !name.is_empty() {
                tools.push(name.to_string());
            }
        }
    }

    if tools.is_empty() { None } else { Some(tools) }
}

/// Extract tool usage guidance from tools.md — everything outside the
/// `## Primary Tools` section. This captures workflow instructions, usage
/// patterns, and constraints that agents should follow when using tools.
pub fn extract_tool_guidance(tools_md: &str) -> String {
    if tools_md.trim().is_empty() {
        return String::new();
    }

    let lines: Vec<&str> = tools_md.lines().collect();

    // Find the Primary Tools section bounds
    let section_start = lines
        .iter()
        .position(|l| l.trim().to_uppercase().starts_with("## PRIMARY TOOLS"));

    let (section_start_idx, section_end_idx) = if let Some(start) = section_start {
        // Find end of section (next ## heading or EOF)
        let end = lines[start + 1..]
            .iter()
            .position(|l| l.trim().starts_with("##"))
            .map(|i| i + start + 1)
            .unwrap_or(lines.len());
        (start, end)
    } else {
        // No Primary Tools section — everything is guidance
        (lines.len(), lines.len())
    };

    // Collect lines outside the Primary Tools section
    let mut guidance_lines: Vec<&str> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if i < section_start_idx || i >= section_end_idx {
            guidance_lines.push(line);
        }
    }

    let result = guidance_lines.join("\n").trim().to_string();
    result
}

// ---------------------------------------------------------------------------
// Memory keys parsing
// ---------------------------------------------------------------------------

fn parse_memory_keys(
    json_str: &str,
    agent_name: &str,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Option<AgentMemoryScope>,
) {
    if json_str.trim().is_empty() {
        return (Vec::new(), Vec::new(), Vec::new(), None);
    }

    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                agent = agent_name,
                error = %e,
                "malformed memory-keys.json, using defaults"
            );
            return (Vec::new(), Vec::new(), Vec::new(), None);
        }
    };

    let recall = extract_string_array(&parsed, "recall_queries");
    let leann = extract_string_array(&parsed, "leann_queries");
    let tags = extract_string_array(&parsed, "tags");

    // Parse optional memory_scope field (values: "user", "project", "local")
    let memory_scope = parsed
        .get("memory_scope")
        .and_then(|v| v.as_str())
        .and_then(|s| {
            serde_json::from_value::<AgentMemoryScope>(serde_json::Value::String(s.to_string()))
                .ok()
        });

    (recall, leann, tags, memory_scope)
}

fn extract_string_array(val: &serde_json::Value, key: &str) -> Vec<String> {
    val.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Meta.json parsing — handles both PRD schema and legacy on-disk schema
// ---------------------------------------------------------------------------

/// Execution config fields extracted from meta.json (not part of AgentMeta).
#[derive(Debug, Default)]
struct ExecConfig {
    model: Option<String>,
    effort: Option<String>,
    max_turns: Option<u32>,
    permission_mode: Option<PermissionMode>,
    background: bool,
    initial_prompt: Option<String>,
    color: Option<String>,
    disallowed_tools: Option<Vec<String>>,
    isolation: Option<String>,
    mcp_servers: Option<Vec<String>>,
    required_mcp_servers: Option<Vec<String>>,
    hooks: Option<serde_json::Value>,
    omit_claude_md: bool,
    skills: Option<Vec<String>>,
    critical_system_reminder: Option<String>,
}

fn parse_meta_json(json_str: &str, agent_name: &str) -> (AgentMeta, ExecConfig) {
    if json_str.trim().is_empty() {
        return (AgentMeta::default(), ExecConfig::default());
    }

    let parsed: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                agent = agent_name,
                error = %e,
                "malformed meta.json, using defaults"
            );
            return (AgentMeta::default(), ExecConfig::default());
        }
    };

    // Try PRD schema first (has "version" as string, "created_at", "updated_at")
    // Fallback to legacy schema (has "version" as int, "created", "last_used")
    let meta = match serde_json::from_value::<AgentMeta>(parsed.clone()) {
        Ok(m) => m,
        Err(_) => {
            // Legacy format — extract what we can
            let mut m = AgentMeta::default();
            if let Some(v) = parsed.get("version") {
                if let Some(n) = v.as_u64() {
                    m.version = format!("{n}.0");
                } else if let Some(s) = v.as_str() {
                    m.version = s.to_string();
                }
            }
            if let Some(n) = parsed.get("invocation_count").and_then(|v| v.as_u64()) {
                m.invocation_count = n;
            }
            if let Some(q) = parsed.get("quality") {
                m.quality = AgentQuality {
                    applied_rate: q
                        .get("applied_rate")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                    completion_rate: q
                        .get("completion_rate")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                };
            }
            if let Some(archived) = parsed.get("archived").and_then(|v| v.as_bool()) {
                m.archived = archived;
            }
            m
        }
    };

    // Extract execution config fields (same key names in both schemas)
    let exec = ExecConfig {
        model: parsed
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from),
        effort: parsed
            .get("effort")
            .and_then(|v| v.as_str())
            .map(String::from),
        max_turns: parsed
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|n| n as u32),
        permission_mode: parsed
            .get("permission_mode")
            .and_then(|v| v.as_str())
            .and_then(PermissionMode::from_str_opt),
        background: parsed
            .get("background")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        initial_prompt: parsed
            .get("initial_prompt")
            .and_then(|v| v.as_str())
            .map(String::from),
        color: parsed
            .get("color")
            .and_then(|v| v.as_str())
            .map(String::from),
        disallowed_tools: parsed.get("disallowed_tools").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        }),
        isolation: parsed
            .get("isolation")
            .and_then(|v| v.as_str())
            .map(String::from),
        mcp_servers: parsed.get("mcp_servers").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        }),
        required_mcp_servers: parsed.get("required_mcp_servers").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        }),
        hooks: parsed.get("hooks").cloned(),
        omit_claude_md: parsed
            .get("omit_claude_md")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        skills: parsed.get("skills").and_then(|v| {
            v.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        }),
        critical_system_reminder: parsed
            .get("critical_system_reminder")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
    };

    (meta, exec)
}

// ---------------------------------------------------------------------------
// Agent hooks parsing (AGT-019)
// ---------------------------------------------------------------------------

/// Parse agent hooks JSON into `(HookEvent, HookConfig)` pairs.
///
/// Expected format (same as Claude Code frontmatter hooks):
/// ```json
/// {
///   "PreToolUse": [{ "type": "command", "command": "check.sh" }],
///   "SessionStart": [{ "type": "command", "command": "setup.sh" }]
/// }
/// ```
///
/// Returns an error if the JSON is not an object, contains unknown event names,
/// or has malformed hook configs.
pub fn parse_agent_hooks(json: &serde_json::Value) -> Result<Vec<(HookEvent, HookConfig)>, String> {
    let map = json
        .as_object()
        .ok_or_else(|| "hooks must be a JSON object".to_string())?;

    let mut result = Vec::new();
    for (event_name, configs_val) in map {
        let event: HookEvent =
            serde_json::from_value(serde_json::Value::String(event_name.clone()))
                .map_err(|e| format!("unknown hook event '{event_name}': {e}"))?;

        let configs: Vec<HookConfig> = serde_json::from_value(configs_val.clone())
            .map_err(|e| format!("malformed hook configs for '{event_name}': {e}"))?;

        for config in configs {
            result.push((event.clone(), config));
        }
    }
    Ok(result)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_agent(dir: &Path, name: &str) {
        let agent_dir = dir.join(name);
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("agent.md"),
            "# Test Agent\n\n## INTENT\nThis agent does testing.\n\n## SCOPE\nStuff\n",
        )
        .unwrap();
        fs::write(agent_dir.join("behavior.md"), "Be careful.\n").unwrap();
        fs::write(agent_dir.join("context.md"), "Context here.\n").unwrap();
        fs::write(
            agent_dir.join("tools.md"),
            "# Tools\n\n## Primary Tools\n- **Read**: read files\n- **Grep**: search\n",
        )
        .unwrap();
        fs::write(
            agent_dir.join("memory-keys.json"),
            r#"{"recall_queries":["testing"],"leann_queries":["test pattern"],"tags":["test"]}"#,
        )
        .unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":5,"quality":{"applied_rate":0.9,"completion_rate":0.8},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();
    }

    #[test]
    fn loads_agent_from_complete_directory() {
        let tmp = TempDir::new().unwrap();
        create_test_agent(tmp.path(), "test-agent");

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert_eq!(agents.len(), 1);

        let agent = &agents[0];
        assert_eq!(agent.agent_type, "test-agent");
        assert_eq!(agent.description, "This agent does testing.");
        assert!(agent.system_prompt.contains("Test Agent"));
        assert!(agent.system_prompt.contains("Be careful."));
        assert!(agent.system_prompt.contains("Context here."));
        assert_eq!(
            agent.allowed_tools,
            Some(vec!["Read".to_string(), "Grep".to_string()])
        );
        assert_eq!(agent.recall_queries, vec!["testing"]);
        assert_eq!(agent.leann_queries, vec!["test pattern"]);
        assert_eq!(agent.tags, vec!["test"]);
        assert_eq!(agent.meta.invocation_count, 5);
        assert_eq!(agent.source, AgentSource::Project);
    }

    #[test]
    fn skips_underscore_directories() {
        let tmp = TempDir::new().unwrap();
        create_test_agent(tmp.path(), "real-agent");
        fs::create_dir_all(tmp.path().join("_template")).unwrap();
        fs::create_dir_all(tmp.path().join("_behavior_schema")).unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::User).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_type, "real-agent");
    }

    #[test]
    fn missing_files_use_defaults() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("minimal");
        fs::create_dir_all(&agent_dir).unwrap();
        // Only create agent.md — everything else missing
        fs::write(agent_dir.join("agent.md"), "# Minimal Agent\n").unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert_eq!(agents.len(), 1);

        let agent = &agents[0];
        assert_eq!(agent.agent_type, "minimal");
        assert!(agent.allowed_tools.is_none()); // No tools.md → all tools
        assert!(agent.recall_queries.is_empty());
        assert_eq!(agent.meta.version, "1.0"); // Default
    }

    #[test]
    fn malformed_meta_json_uses_defaults() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("bad-meta");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(agent_dir.join("meta.json"), "not valid json!!!").unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].meta.version, "1.0"); // Default
        assert_eq!(agents[0].meta.invocation_count, 0);
    }

    #[test]
    fn malformed_memory_keys_uses_defaults() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("bad-memory");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(agent_dir.join("memory-keys.json"), "{broken").unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert_eq!(agents.len(), 1);
        assert!(agents[0].recall_queries.is_empty());
        assert!(agents[0].leann_queries.is_empty());
    }

    #[test]
    fn legacy_meta_json_version_as_integer() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("legacy");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Legacy\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":1,"created":"2026-01-01T00:00:00Z","invocation_count":42,"quality":{"applied_rate":0.95,"completion_rate":0.88},"evolution_history_last_10":[]}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert_eq!(agents[0].meta.version, "1.0");
        assert_eq!(agents[0].meta.invocation_count, 42);
    }

    #[test]
    fn extract_description_from_intent() {
        let md = "# Agent\n\n## INTENT\nFirst paragraph here.\n\n## SCOPE\nOther stuff\n";
        assert_eq!(extract_description(md), "First paragraph here.");
    }

    #[test]
    fn extract_description_multiline_paragraph() {
        let md = "# Agent\n\n## INTENT\nLine one of\nthe description.\n\n## SCOPE\n";
        assert_eq!(extract_description(md), "Line one of the description.");
    }

    #[test]
    fn extract_description_no_intent_uses_first_line() {
        let md = "# My Great Agent\n\nSome content\n";
        assert_eq!(extract_description(md), "My Great Agent");
    }

    #[test]
    fn extract_tools_from_primary_section() {
        let md = "# Tools\n\n## Primary Tools\n- **Read**: read files\n- **Grep**: search\n\n## Workflow\nStuff\n";
        let tools = extract_tools(md);
        assert_eq!(tools, Some(vec!["Read".to_string(), "Grep".to_string()]));
    }

    #[test]
    fn extract_tools_none_when_no_section() {
        let md = "# Tools\n\nSome general info\n";
        assert!(extract_tools(md).is_none());
    }

    #[test]
    fn extract_tools_none_when_section_empty() {
        let md = "# Tools\n\n## Primary Tools\n\n## Workflow\nStuff\n";
        assert!(extract_tools(md).is_none());
    }

    #[test]
    fn empty_directory_returns_empty_vec() {
        let tmp = TempDir::new().unwrap();
        let agents = load_custom_agents(tmp.path(), AgentSource::User).unwrap();
        assert!(agents.is_empty());
    }

    // -----------------------------------------------------------------------
    // Truncation tests (AGT-016)
    // -----------------------------------------------------------------------

    #[test]
    fn truncate_under_budget_preserves_all() {
        let result = truncate_agent_prompt("agent", "behavior", "context", 100);
        assert_eq!(result, "agent\n\nbehavior\n\ncontext");
    }

    #[test]
    fn truncate_context_first_when_over_budget() {
        let agent = "A".repeat(50);
        let behavior = "B".repeat(50);
        let context = "C".repeat(500);
        // Budget: 40 tokens = 160 chars. agent(50) + sep(4) = 54, remaining = 106
        // behavior(50) fits, context budget = 106-50 = 56, context(500) > 56 → truncated
        let result = truncate_agent_prompt(&agent, &behavior, &context, 40);
        assert!(result.starts_with(&agent));
        assert!(result.contains(&behavior));
        // full context should NOT be present
        assert!(!result.contains(&context));
        // but truncated marker should be
        assert!(result.contains("... [truncated]"));
    }

    #[test]
    fn truncate_behavior_when_context_gone_not_enough() {
        let agent = "A".repeat(50);
        let behavior = "B".repeat(500);
        let context = "C".repeat(500);
        // Budget: 20 tokens = 80 chars. agent(50) + sep(4) = 54, remaining = 26
        // context trimmed first, behavior 500 > 26, so behavior also trimmed
        let result = truncate_agent_prompt(&agent, &behavior, &context, 20);
        assert!(result.starts_with(&agent));
        assert!(result.contains("... [truncated]"));
        assert!(!result.contains(&behavior));
    }

    #[test]
    fn truncate_agent_md_never_trimmed() {
        // agent_md bigger than total budget — still not trimmed
        let agent = "A".repeat(200);
        let behavior = "B".repeat(10);
        let context = "C".repeat(10);
        // Budget: 10 tokens = 40 chars. agent alone is 200
        let result = truncate_agent_prompt(&agent, &behavior, &context, 10);
        // agent_md is always included fully
        assert!(result.contains(&agent));
    }

    #[test]
    fn truncate_adds_truncated_marker() {
        let agent = "A".repeat(10);
        let behavior = "B".repeat(10);
        let context = "C".repeat(500);
        // Budget: 15 tokens = 60 chars. Total = 10+10+500+4 = 524, needs truncation
        let result = truncate_agent_prompt(&agent, &behavior, &context, 15);
        assert!(result.contains("... [truncated]"));
    }

    #[test]
    fn truncate_empty_sections_handled() {
        let result = truncate_agent_prompt("agent", "", "", 100);
        assert_eq!(result, "agent");
    }

    #[test]
    fn truncate_wired_into_assemble() {
        // Verify assemble_system_prompt calls truncation (under budget = passthrough)
        let prompt = assemble_system_prompt("# Agent", "rules", "ctx");
        assert_eq!(prompt, "# Agent\n\nrules\n\nctx");
    }

    #[test]
    fn exec_config_parsed_from_meta_json() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("configured");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"model":"opus","effort":"high","max_turns":20,"background":true}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        let agent = &agents[0];
        assert_eq!(agent.model.as_deref(), Some("opus"));
        assert_eq!(agent.effort.as_deref(), Some("high"));
        assert_eq!(agent.max_turns, Some(20));
        assert!(agent.background);
    }

    #[test]
    fn isolation_parsed_from_meta_json() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("isolated");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"isolation":"worktree"}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert_eq!(agents[0].isolation.as_deref(), Some("worktree"));
    }

    #[test]
    fn mcp_servers_parsed_from_meta_json() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("mcp-agent");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"mcp_servers":["github","slack"],"required_mcp_servers":["github"]}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert_eq!(
            agents[0].mcp_servers,
            Some(vec!["github".into(), "slack".into()])
        );
        assert_eq!(agents[0].required_mcp_servers, Some(vec!["github".into()]));
    }

    #[test]
    fn mcp_servers_none_when_not_in_meta() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("no-mcp");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(agents[0].mcp_servers.is_none());
        assert!(agents[0].required_mcp_servers.is_none());
    }

    #[test]
    fn isolation_none_when_not_in_meta() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("no-isolation");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(agents[0].isolation.is_none());
    }

    // -----------------------------------------------------------------------
    // Session-scoped hooks tests (AGT-019)
    // -----------------------------------------------------------------------

    #[test]
    fn hooks_parsed_from_meta_json() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("hooked");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"hooks":{"PreToolUse":[{"type":"command","command":"check.sh"}]}}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        let hooks = agents[0].hooks.as_ref().expect("hooks should be Some");
        assert!(hooks.is_object());
        assert!(hooks.get("PreToolUse").is_some());
    }

    #[test]
    fn hooks_none_when_not_in_meta_json() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("no-hooks");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(agents[0].hooks.is_none());
    }

    #[test]
    fn parse_agent_hooks_valid_single_event() {
        use crate::hooks::{HookCommandType, HookEvent};
        let json: serde_json::Value = serde_json::json!({
            "PreToolUse": [
                {"type": "command", "command": "check.sh"},
                {"type": "command", "command": "lint.sh", "timeout": 30}
            ]
        });
        let hooks = parse_agent_hooks(&json).unwrap();
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0].0, HookEvent::PreToolUse);
        assert_eq!(hooks[0].1.hook_type, HookCommandType::Command);
        assert_eq!(hooks[0].1.command, "check.sh");
        assert_eq!(hooks[1].0, HookEvent::PreToolUse);
        assert_eq!(hooks[1].1.command, "lint.sh");
        assert_eq!(hooks[1].1.timeout, Some(30));
    }

    #[test]
    fn parse_agent_hooks_multiple_events() {
        use crate::hooks::HookEvent;
        let json: serde_json::Value = serde_json::json!({
            "SessionStart": [{"type": "command", "command": "setup.sh"}],
            "PreToolUse": [{"type": "command", "command": "guard.sh"}]
        });
        let hooks = parse_agent_hooks(&json).unwrap();
        assert_eq!(hooks.len(), 2);
        let events: Vec<&HookEvent> = hooks.iter().map(|(e, _)| e).collect();
        assert!(events.contains(&&HookEvent::SessionStart));
        assert!(events.contains(&&HookEvent::PreToolUse));
    }

    #[test]
    fn parse_agent_hooks_empty_object() {
        let json: serde_json::Value = serde_json::json!({});
        let hooks = parse_agent_hooks(&json).unwrap();
        assert!(hooks.is_empty());
    }

    #[test]
    fn parse_agent_hooks_not_object_returns_error() {
        let json: serde_json::Value = serde_json::json!("not an object");
        let err = parse_agent_hooks(&json).unwrap_err();
        assert!(err.contains("must be a JSON object"));
    }

    #[test]
    fn parse_agent_hooks_unknown_event_returns_error() {
        let json: serde_json::Value = serde_json::json!({
            "BogusEvent": [{"type": "command", "command": "x.sh"}]
        });
        let err = parse_agent_hooks(&json).unwrap_err();
        assert!(err.contains("unknown hook event"));
    }

    // -----------------------------------------------------------------------
    // omitClaudeMd tests (AGT-021)
    // -----------------------------------------------------------------------

    #[test]
    fn omit_claude_md_parsed_from_meta_json() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("readonly");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"omit_claude_md":true}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(agents[0].omit_claude_md);
    }

    #[test]
    fn omit_claude_md_defaults_to_false() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("default");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(!agents[0].omit_claude_md);
    }

    #[test]
    fn parse_agent_hooks_malformed_config_returns_error() {
        let json: serde_json::Value = serde_json::json!({
            "PreToolUse": [{"missing_type_field": true}]
        });
        let err = parse_agent_hooks(&json).unwrap_err();
        assert!(err.contains("malformed hook configs"));
    }

    // -----------------------------------------------------------------------
    // Skills preloading tests (AGT-020)
    // -----------------------------------------------------------------------

    #[test]
    fn skills_parsed_from_meta_json() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("skilled");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"skills":["commit","review-pr"]}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        let skills = agents[0].skills.as_ref().expect("skills should be Some");
        assert_eq!(skills, &vec!["commit".to_string(), "review-pr".to_string()]);
    }

    #[test]
    fn skills_none_when_not_in_meta_json() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("no-skills");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(agents[0].skills.is_none());
    }

    // -----------------------------------------------------------------------
    // criticalSystemReminder tests (AGT-022)
    // -----------------------------------------------------------------------

    #[test]
    fn critical_system_reminder_parsed_from_meta_json() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("reminded");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"critical_system_reminder":"NEVER edit files"}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert_eq!(
            agents[0].critical_system_reminder.as_deref(),
            Some("NEVER edit files")
        );
    }

    #[test]
    fn critical_system_reminder_none_when_absent() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("no-reminder");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(agents[0].critical_system_reminder.is_none());
    }

    #[test]
    fn critical_system_reminder_empty_string_treated_as_none() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("empty-reminder");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"critical_system_reminder":""}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(agents[0].critical_system_reminder.is_none());
    }

    #[test]
    fn skills_empty_array_parsed_as_some_empty() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("empty-skills");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false,"skills":[]}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        let skills = agents[0].skills.as_ref().expect("skills should be Some");
        assert!(skills.is_empty());
    }

    // -----------------------------------------------------------------------
    // memory_scope parsing tests (AGT-002)
    // -----------------------------------------------------------------------

    #[test]
    fn memory_scope_parsed_from_memory_keys_json() {
        use super::super::definition::AgentMemoryScope;
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("scoped");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("memory-keys.json"),
            r#"{"recall_queries":[],"leann_queries":[],"tags":[],"memory_scope":"project"}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert_eq!(agents[0].memory_scope, Some(AgentMemoryScope::Project));
    }

    #[test]
    fn memory_scope_none_when_absent_in_memory_keys() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("no-scope");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("memory-keys.json"),
            r#"{"recall_queries":["test"],"leann_queries":[],"tags":[]}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(agents[0].memory_scope.is_none());
    }

    #[test]
    fn memory_scope_none_when_memory_keys_missing() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("no-keys");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        // No memory-keys.json at all

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(agents[0].memory_scope.is_none());
    }

    #[test]
    fn memory_scope_invalid_value_defaults_to_none() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("bad-scope");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
        fs::write(
            agent_dir.join("memory-keys.json"),
            r#"{"recall_queries":[],"leann_queries":[],"tags":[],"memory_scope":"invalid_value"}"#,
        )
        .unwrap();

        let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
        assert!(agents[0].memory_scope.is_none());
    }

    #[test]
    fn memory_scope_all_variants() {
        use super::super::definition::AgentMemoryScope;
        for (scope_str, expected) in [
            ("user", AgentMemoryScope::User),
            ("project", AgentMemoryScope::Project),
            ("local", AgentMemoryScope::Local),
        ] {
            let tmp = TempDir::new().unwrap();
            let agent_dir = tmp.path().join("scope-test");
            fs::create_dir_all(&agent_dir).unwrap();
            fs::write(agent_dir.join("agent.md"), "# Agent\n").unwrap();
            fs::write(
                agent_dir.join("memory-keys.json"),
                format!(
                    r#"{{"recall_queries":[],"leann_queries":[],"tags":[],"memory_scope":"{}"}}"#,
                    scope_str
                ),
            )
            .unwrap();

            let agents = load_custom_agents(tmp.path(), AgentSource::Project).unwrap();
            assert_eq!(
                agents[0].memory_scope,
                Some(expected),
                "memory_scope '{}' should parse correctly",
                scope_str
            );
        }
    }

    // -----------------------------------------------------------------------
    // Integration tests: load real agents from disk (AGT-002)
    // -----------------------------------------------------------------------

    #[test]
    fn loads_all_9_real_agents_from_custom_dir() {
        let custom_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.archon/agents/custom");
        if !custom_dir.exists() {
            eprintln!(
                "Skipping: .archon/agents/custom/ not found at {:?}",
                custom_dir
            );
            return;
        }

        let agents = load_custom_agents(&custom_dir, AgentSource::Project).unwrap();

        // Should load at least 9 agents (may be more if new ones added)
        assert!(
            agents.len() >= 9,
            "Expected at least 9 agents, got {}",
            agents.len()
        );

        // All agents should have non-empty agent_type
        for agent in &agents {
            assert!(
                !agent.agent_type.is_empty(),
                "agent_type should not be empty"
            );
        }

        // None should start with '_'
        for agent in &agents {
            assert!(
                !agent.agent_type.starts_with('_'),
                "agent '{}' starts with _ and should have been skipped",
                agent.agent_type
            );
        }
    }

    #[test]
    fn code_reviewer_loads_with_nonempty_fields() {
        let custom_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.archon/agents/custom");
        if !custom_dir.exists() {
            eprintln!(
                "Skipping: .archon/agents/custom/ not found at {:?}",
                custom_dir
            );
            return;
        }

        let agents = load_custom_agents(&custom_dir, AgentSource::Project).unwrap();
        let reviewer = agents
            .iter()
            .find(|a| a.agent_type == "code-reviewer")
            .expect("code-reviewer agent should exist");

        assert!(
            !reviewer.system_prompt.is_empty(),
            "code-reviewer system_prompt should be non-empty"
        );
        assert!(
            !reviewer.description.is_empty(),
            "code-reviewer description should be non-empty"
        );
        assert!(
            reviewer.allowed_tools.is_some(),
            "code-reviewer should have explicit allowed_tools"
        );
        let tools = reviewer.allowed_tools.as_ref().unwrap();
        assert!(
            !tools.is_empty(),
            "code-reviewer allowed_tools should not be empty"
        );
    }

    // -----------------------------------------------------------------------
    // Tool guidance extraction tests (AGT-002)
    // -----------------------------------------------------------------------

    #[test]
    fn extract_tool_guidance_from_tools_md() {
        let md = "# Tool Workflow\n\nAlways read before editing.\n\n## Primary Tools\n- **Read**: read\n- **Edit**: edit\n\n## Usage Notes\nBe careful with Edit.\n";
        let guidance = extract_tool_guidance(md);
        assert!(guidance.contains("Always read before editing."));
        assert!(guidance.contains("Be careful with Edit."));
        assert!(!guidance.contains("- **Read**"));
    }

    #[test]
    fn extract_tool_guidance_empty_tools_md() {
        assert!(extract_tool_guidance("").is_empty());
        assert!(extract_tool_guidance("   \n  ").is_empty());
    }
}
