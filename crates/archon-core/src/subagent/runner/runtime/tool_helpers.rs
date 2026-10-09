use super::*;

/// These shared guard markers identify results returned without admitting
/// any tool work. Ordinary tool errors still count as attempted work.
pub(super) fn is_guard_refusal(result: &ToolResult) -> bool {
    if !result.is_error {
        return false;
    }
    let content = result
        .content
        .trim_start()
        .strip_prefix("Error: ")
        .unwrap_or(result.content.trim_start());
    [
        archon_tools::workflow_read_guard::READ_CEILING_MARKER,
        archon_tools::workflow_read_guard::READ_WALL_THRASH_MARKER,
        archon_tools::workflow_read_guard::REPEATED_FAILURE_MARKER,
    ]
    .iter()
    .any(|marker| content.starts_with(marker))
}

pub(super) fn tool_allows_empty_input(runner: &SubagentRunner, name: &str) -> bool {
    runner
        .registry
        .lookup(name)
        .map(|tool_arc| {
            crate::agent::tool_input_json::schema_allows_empty_input(&tool_arc.input_schema())
        })
        .unwrap_or(false)
}
