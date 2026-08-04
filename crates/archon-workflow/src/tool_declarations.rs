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
