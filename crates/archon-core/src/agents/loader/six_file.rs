use std::path::Path;

use tracing::warn;

use super::meta::{parse_memory_keys, parse_meta_json};
use super::prompt::{
    assemble_system_prompt, extract_description, extract_tool_guidance, extract_tools,
};
use crate::agents::definition::{AgentSource, CustomAgentDefinition};

pub(super) fn load_single_agent(
    dir: &Path,
    name: &str,
    source: AgentSource,
) -> CustomAgentDefinition {
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
