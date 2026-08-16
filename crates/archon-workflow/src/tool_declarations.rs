//! The tool-declaration key vocabulary, and the scrub that removes it.
//!
//! Two places read the same list of keys and must never drift: the binary's
//! `allowed_mcp_tools`, which collects declared tools out of a stage input to
//! decide what MCP binding permits, and [`strip_tool_declarations`], which
//! removes them from an agent-authored fan-out item before the host stamps the
//! authoritative set derived from the task universe.
//!
//! They live together here so "the two lists must never drift" is a structural
//! fact rather than a comment: there is one list.

/// Every key treated as a tool declaration.
pub const TOOL_DECLARATION_FIELDS: &[&str] = &[
    "required_tools",
    "requiredTools",
    "tool_requirements",
    "toolRequirements",
    "mcp_tools",
    "mcpTools",
];

/// Whether `key` declares tools.
pub fn is_tool_field(key: &str) -> bool {
    TOOL_DECLARATION_FIELDS.contains(&key)
}

/// Reduce a declared tool name to the bare name a grant and a command both
/// speak: `mcp__provider__quote_get` and `mcp_action:quote_get` alike become
/// `quote_get`.
///
/// Two qualifier conventions are in use, and this must reduce BOTH or the two
/// sides of a comparison never meet. That is not hypothetical: the binding
/// filter intersected task-declared names against project-permitted ones, each
/// side reduced by a copy of this function that knew only `mcp__server__`. A
/// task declaring `mcp_action:tv_health_check` kept its qualifier, the
/// permitted `mcp__provider__x_health_check` reduced to `x_health_check`,
/// the intersection came out empty, and the stage fell back to the hardcoded
/// tool list — so no agent on that task ever received an MCP tool, while the
/// separate exercise gate failed it for never using one.
pub fn raw_tool_name(name: &str) -> &str {
    let name = name.trim();
    if let Some(raw) = name
        .strip_prefix("mcp__")
        .and_then(|suffix| suffix.split_once("__"))
        .map(|(_, raw)| raw)
    {
        return raw.trim();
    }
    // Any `mcp*:` qualifier, so a new spelling cannot silently break binding.
    if let Some((qualifier, raw)) = name.split_once(':')
        && qualifier.to_ascii_lowercase().starts_with("mcp")
    {
        return raw.trim();
    }
    name
}

/// Remove every tool-declaration key at EVERY level of a value.
///
/// `allowed_mcp_tools` (and the write no-op guard) scan the whole input
/// recursively, so a shallow strip leaves a nested `{...: {mcp_tools: [...]}}`
/// forgery reachable. Applied to agent-authored branch items so only
/// host-stamped, task-universe-derived tools can ever bind.
pub fn strip_tool_declarations(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            object.retain(|key, _| !is_tool_field(key));
            for child in object.values_mut() {
                strip_tool_declarations(child);
            }
        }
        serde_json::Value::Array(values) => {
            for child in values.iter_mut() {
                strip_tool_declarations(child);
            }
        }
        _ => {}
    }
}
