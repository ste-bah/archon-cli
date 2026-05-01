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

/// Load all plugin-provided agents from `plugins_root`.
///
/// Scans `<plugins_root>/<plugin_name>/agents/<agent_name>/` for the 6-file
/// agent structure. Plugin directories whose name starts with `_` are
/// skipped (same convention as [`load_custom_agents`]). Agent subdirectories
/// whose name starts with `_` are also skipped. Loaded agents have their
/// `agent_type` prefixed with `<plugin_name>:` and carry
/// [`AgentSource::Plugin`]`(plugin_name)` as provenance.
///
/// A non-existent `plugins_root` returns `Ok(vec![])` — plugin roots are
/// optional, not an error condition. A plugin directory without an
/// `agents/` subdir is silently skipped (the plugin may provide only
/// skills, hooks, or other resources).
pub fn load_plugin_agents(
    plugins_root: &Path,
) -> Result<Vec<CustomAgentDefinition>, AgentLoadError> {
    if !plugins_root.is_dir() {
        return Ok(Vec::new());
    }

    let entries = std::fs::read_dir(plugins_root).map_err(|e| AgentLoadError::ReadDir {
        path: plugins_root.display().to_string(),
        source: e,
    })?;

    let mut agents = Vec::new();

    for entry in entries.flatten() {
        let plugin_path = entry.path();
        if !plugin_path.is_dir() {
            continue;
        }
        let plugin_name = match plugin_path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip private/template/archived plugin dirs (same rule as custom agents).
        if plugin_name.starts_with('_') {
            continue;
        }

        // Agents live at <plugin>/agents/<agent_name>/
        let agents_dir = plugin_path.join("agents");
        if !agents_dir.is_dir() {
            // Plugin without an agents/ subdir: plugin may provide only
            // skills/hooks/commands/etc. Not an error.
            continue;
        }

        let agent_entries = match std::fs::read_dir(&agents_dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    plugin = plugin_name,
                    dir = %agents_dir.display(),
                    error = %e,
                    "failed to read plugin agents dir; skipping plugin"
                );
                continue;
            }
        };

        for agent_entry in agent_entries.flatten() {
            let agent_path = agent_entry.path();
            if !agent_path.is_dir() {
                continue;
            }
            let agent_name = match agent_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Inherit the _-prefix skip rule for agent subdirs too.
            if agent_name.starts_with('_') {
                continue;
            }

            let mut def = load_single_agent(
                &agent_path,
                &agent_name,
                AgentSource::Plugin(plugin_name.clone()),
            );
            // Namespace the agent_type with the plugin name so it can't
            // collide with bare custom agents of the same local name.
            def.agent_type = format!("{plugin_name}:{agent_name}");
            agents.push(def);
        }
    }

    Ok(agents)
}

// ---------------------------------------------------------------------------
// Flat-file YAML-frontmatter agent loader (v0.1.11)
// ---------------------------------------------------------------------------

/// Extract YAML frontmatter and body from a markdown string.
///
/// Frontmatter is delimited by `---` lines. Returns `(frontmatter, body)`
/// or `None` if the file does not start with `---` or has no closing `---`.
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
        serde_yml::from_str(&frontmatter).map_err(|e| format!("YAML parse error: {e}"))?;

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

    Ok(Some(CustomAgentDefinition {
        agent_type,
        system_prompt: body.trim().to_string(),
        description,
        allowed_tools,
        disallowed_tools: None,
        tool_guidance: String::new(),
        model,
        effort: None,
        max_turns: None,
        permission_mode: None,
        background: false,
        initial_prompt: None,
        color,
        memory_scope: None,
        recall_queries: Vec::new(),
        leann_queries: Vec::new(),
        tags,
        source: source.clone(),
        meta: AgentMeta::default(),
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

// ---------------------------------------------------------------------------
// Single agent loader (6-file format)
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
                .split(['*', ':', ' '])
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

    
    guidance_lines.join("\n").trim().to_string()
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

    // -----------------------------------------------------------------------
    // G5 — plugin agent loader tests
    // -----------------------------------------------------------------------

    /// Create a minimal plugin agent fixture at
    /// `<plugins_root>/<plugin>/agents/<agent>/` with the 6-file structure.
    /// `agent_md_body` allows tests to distinguish two otherwise-identical
    /// agents (used by the priority collision test).
    fn create_plugin_agent(plugins_root: &Path, plugin: &str, agent: &str, agent_md_body: &str) {
        let agent_dir = plugins_root.join(plugin).join("agents").join(agent);
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(
            agent_dir.join("agent.md"),
            format!("# {agent}\n\n## INTENT\n{agent_md_body}\n"),
        )
        .unwrap();
        fs::write(agent_dir.join("behavior.md"), "Be careful.\n").unwrap();
        fs::write(agent_dir.join("context.md"), "Plugin context.\n").unwrap();
        fs::write(
            agent_dir.join("tools.md"),
            "# Tools\n\n## Primary Tools\n- **Read**: read files\n",
        )
        .unwrap();
        fs::write(
            agent_dir.join("memory-keys.json"),
            r#"{"recall_queries":[],"leann_queries":[],"tags":[]}"#,
        )
        .unwrap();
        fs::write(
            agent_dir.join("meta.json"),
            r#"{"version":"1.0","created_at":"2026-04-01T00:00:00Z","updated_at":"2026-04-01T00:00:00Z","invocation_count":0,"quality":{"applied_rate":0.0,"completion_rate":0.0},"evolution_history":[],"archived":false}"#,
        )
        .unwrap();
    }

    #[test]
    fn load_plugin_agents_discovers_single_plugin_agent() {
        let tmp = TempDir::new().unwrap();
        create_plugin_agent(tmp.path(), "foo", "bar", "Bar agent body.");

        let agents = load_plugin_agents(tmp.path()).unwrap();

        assert_eq!(agents.len(), 1, "should discover exactly one plugin agent");
        let bar = &agents[0];
        assert_eq!(
            bar.agent_type, "foo:bar",
            "agent_type must be prefixed with plugin name"
        );
        assert_eq!(
            bar.source,
            AgentSource::Plugin("foo".to_string()),
            "source must be Plugin(plugin_name)"
        );
        assert!(
            bar.description.contains("Bar agent body"),
            "description should come from agent.md INTENT"
        );
    }

    #[test]
    fn load_plugin_agents_skips_underscore_plugin_dirs() {
        let tmp = TempDir::new().unwrap();
        create_plugin_agent(tmp.path(), "_internal", "bar", "Internal bar.");
        create_plugin_agent(tmp.path(), "real", "bar", "Real bar.");

        let agents = load_plugin_agents(tmp.path()).unwrap();

        assert_eq!(agents.len(), 1, "_-prefixed plugin dirs must be skipped");
        assert_eq!(agents[0].agent_type, "real:bar");
    }

    #[test]
    fn load_plugin_agents_skips_underscore_agent_dirs() {
        let tmp = TempDir::new().unwrap();
        create_plugin_agent(tmp.path(), "foo", "_template", "Template.");
        create_plugin_agent(tmp.path(), "foo", "real", "Real agent.");

        let agents = load_plugin_agents(tmp.path()).unwrap();

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_type, "foo:real");
    }

    #[test]
    fn load_plugin_agents_nonexistent_root_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");
        let agents = load_plugin_agents(&nonexistent).unwrap();
        assert!(agents.is_empty());
    }

    #[test]
    fn load_plugin_agents_empty_plugins_dir_returns_empty() {
        let tmp = TempDir::new().unwrap();
        // Create an empty plugins root.
        fs::create_dir_all(tmp.path()).unwrap();
        let agents = load_plugin_agents(tmp.path()).unwrap();
        assert!(agents.is_empty());
    }

    #[test]
    fn load_plugin_agents_plugin_without_agents_dir_skipped() {
        let tmp = TempDir::new().unwrap();
        // A plugin with no agents/ subdir should be silently skipped.
        fs::create_dir_all(tmp.path().join("no-agents")).unwrap();
        // Plus a real one to ensure the loop continues.
        create_plugin_agent(tmp.path(), "has-agents", "bar", "Bar.");

        let agents = load_plugin_agents(tmp.path()).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_type, "has-agents:bar");
    }

    #[test]
    fn load_plugin_agents_multiple_plugins_and_agents() {
        let tmp = TempDir::new().unwrap();
        create_plugin_agent(tmp.path(), "alpha", "one", "alpha one");
        create_plugin_agent(tmp.path(), "alpha", "two", "alpha two");
        create_plugin_agent(tmp.path(), "beta", "one", "beta one");

        let mut agents = load_plugin_agents(tmp.path()).unwrap();
        agents.sort_by(|a, b| a.agent_type.cmp(&b.agent_type));

        assert_eq!(agents.len(), 3);
        assert_eq!(agents[0].agent_type, "alpha:one");
        assert_eq!(agents[0].source, AgentSource::Plugin("alpha".into()));
        assert_eq!(agents[1].agent_type, "alpha:two");
        assert_eq!(agents[1].source, AgentSource::Plugin("alpha".into()));
        assert_eq!(agents[2].agent_type, "beta:one");
        assert_eq!(agents[2].source, AgentSource::Plugin("beta".into()));
    }

    // -----------------------------------------------------------------------
    // Flat-file agent loader tests (v0.1.11)
    // -----------------------------------------------------------------------

    #[test]
    fn flat_file_loader_finds_basic_agent() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("over-engineering-therapist.md"),
            "---\nname: over-engineering-therapist\ndescription: Therapeutic code reviewer\n\
             tools: Read, Grep, Glob, Bash\nmodel: sonnet\ncolor: teal\n---\n\n\
             # Over-Engineering Therapist\n\nYou are a specialized therapeutic code reviewer.\n",
        )
        .unwrap();

        let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
        assert_eq!(agents.len(), 1);
        let a = &agents[0];
        assert_eq!(a.agent_type, "over-engineering-therapist");
        assert_eq!(a.description, "Therapeutic code reviewer");
        assert!(
            a.system_prompt
                .contains("You are a specialized therapeutic code reviewer")
        );
        assert_eq!(a.model.as_deref(), Some("sonnet"));
        assert_eq!(a.color.as_deref(), Some("teal"));
        assert_eq!(a.source, AgentSource::Project);
    }

    #[test]
    fn flat_file_loader_recursive_subdirs() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(agents_dir.join("templates")).unwrap();
        fs::write(
            agents_dir.join("templates/sub-agent.md"),
            "---\nname: sub-agent\ndescription: A subdirectory agent\n---\n\nSub agent body.\n",
        )
        .unwrap();
        fs::write(
            agents_dir.join("top-agent.md"),
            "---\nname: top-agent\ndescription: A top-level agent\n---\n\nTop agent body.\n",
        )
        .unwrap();

        let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
        let names: Vec<&str> = agents.iter().map(|a| a.agent_type.as_str()).collect();
        assert!(
            names.contains(&"sub-agent"),
            "sub-agent from subdir must be found; got {:?}",
            names
        );
        assert!(names.contains(&"top-agent"), "top-agent must be found");
        assert_eq!(agents.len(), 2);
    }

    #[test]
    fn flat_file_loader_skips_custom_subdir() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(agents_dir.join("custom")).unwrap();
        fs::write(
            agents_dir.join("custom/should-skip.md"),
            "---\nname: should-skip\ndescription: This is in custom/\n---\n\nBody.\n",
        )
        .unwrap();
        fs::write(
            agents_dir.join("top.md"),
            "---\nname: top\ndescription: Top-level agent\n---\n\nTop body.\n",
        )
        .unwrap();

        let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
        let names: Vec<&str> = agents.iter().map(|a| a.agent_type.as_str()).collect();
        assert!(names.contains(&"top"), "top agent should be loaded");
        assert!(!names.contains(&"should-skip"), "custom/ must be skipped");
        assert_eq!(agents.len(), 1);
    }

    #[test]
    fn flat_file_loader_skips_no_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("README.md"),
            "# README\n\nThis is not an agent — just a readme.\n",
        )
        .unwrap();
        fs::write(
            agents_dir.join("real-agent.md"),
            "---\nname: real-agent\ndescription: A real one\n---\n\nReal body.\n",
        )
        .unwrap();

        let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
        let names: Vec<&str> = agents.iter().map(|a| a.agent_type.as_str()).collect();
        assert!(names.contains(&"real-agent"), "real-agent should be loaded");
        assert!(
            !names.contains(&"README"),
            "README (no frontmatter) must be skipped"
        );
        assert_eq!(agents.len(), 1);
    }

    #[test]
    fn flat_file_loader_malformed_frontmatter_logs_skips() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("broken.md"),
            "---\n{this is not valid yaml]]]\n---\n\nBody.\n",
        )
        .unwrap();
        fs::write(
            agents_dir.join("good.md"),
            "---\nname: good\ndescription: Works fine\n---\n\nGood body.\n",
        )
        .unwrap();

        let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
        let names: Vec<&str> = agents.iter().map(|a| a.agent_type.as_str()).collect();
        assert!(names.contains(&"good"), "good agent must still be loaded");
        assert!(!names.contains(&"broken"), "malformed YAML must be skipped");
        assert_eq!(agents.len(), 1);
    }

    #[test]
    fn flat_file_loader_missing_root_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let nonexistent = tmp.path().join("does-not-exist");
        let agents = load_flat_file_agents(&nonexistent, AgentSource::Project).unwrap();
        assert!(agents.is_empty());
    }

    #[test]
    fn flat_file_loader_tools_csv() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("tooled.md"),
            "---\nname: tooled\ndescription: Has tools\n\
             tools: Read, Grep, Glob, Bash\n---\n\nBody.\n",
        )
        .unwrap();

        let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
        let tools = agents[0].allowed_tools.as_ref().unwrap();
        assert_eq!(tools, &vec!["Read", "Grep", "Glob", "Bash"]);
    }

    #[test]
    fn flat_file_loader_tools_yaml_array() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("tooled.yaml.md"),
            "---\nname: tooled\ndescription: Has tools as array\n\
             tools:\n  - Read\n  - Grep\n  - Glob\n  - Bash\n---\n\nBody.\n",
        )
        .unwrap();

        let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
        let tools = agents[0].allowed_tools.as_ref().unwrap();
        assert_eq!(tools, &vec!["Read", "Grep", "Glob", "Bash"]);
    }

    #[test]
    fn flat_file_loader_filename_stem_fallback() {
        let tmp = TempDir::new().unwrap();
        let agents_dir = tmp.path().join("agents");
        fs::create_dir_all(&agents_dir).unwrap();
        fs::write(
            agents_dir.join("nameless.md"),
            "---\ndescription: No name field here\n---\n\nBody text.\n",
        )
        .unwrap();

        let agents = load_flat_file_agents(&agents_dir, AgentSource::Project).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_type, "nameless");
        assert_eq!(agents[0].description, "No name field here");
    }
}
