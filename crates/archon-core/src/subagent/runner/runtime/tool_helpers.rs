use super::*;

/// These shared guard markers identify results returned without admitting
/// any tool work. Ordinary tool errors still count as attempted work.
pub(super) fn is_guard_refusal(result: &ToolResult) -> bool {
    if !result.is_error {
        return false;
    }
    let content = result.content.trim_start();
    let content = content.strip_prefix("Error: ").unwrap_or(content);
    content.starts_with(archon_tools::tool::TOOL_REFUSAL_MARKER)
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

#[cfg(test)]
mod tests {
    use super::is_guard_refusal;
    use archon_tools::tool::ToolResult;

    #[test]
    fn every_guard_refusal_kind_shares_one_class_and_ordinary_errors_remain_progress() {
        for reason in [
            "read ceiling refusal",
            "freshness refusal",
            "sandbox refusal",
            "tool-run admission refusal",
            "workflow guard refusal",
        ] {
            assert!(is_guard_refusal(&ToolResult::refusal(reason)), "{reason}");
        }
        assert!(!is_guard_refusal(&ToolResult::error(
            "ordinary command failed"
        )));
    }
}
