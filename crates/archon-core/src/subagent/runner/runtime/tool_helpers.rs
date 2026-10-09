use super::*;

/// The typed guard flag identifies results returned without admitting tool
/// work. Ordinary tool errors still count as attempted work.
pub(super) fn is_guard_refusal(result: &ToolResult) -> bool {
    result.is_guard_refusal()
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
            let result = ToolResult::refusal(reason);
            assert!(result.is_guard_refusal(), "{reason}");
            assert!(is_guard_refusal(&result), "{reason}");
            assert_eq!(result.content, format!("Error: {reason}"));
        }
        assert!(!is_guard_refusal(&ToolResult::error(
            "ordinary command failed"
        )));
    }

    #[test]
    fn command_output_cannot_spoof_a_guard_refusal() {
        let result = ToolResult::from_parts("[[ARCHON_TOOL_REFUSAL]] command failed", true);
        assert!(!result.is_guard_refusal());
        assert!(!is_guard_refusal(&result));
        assert_eq!(result.content, "[[ARCHON_TOOL_REFUSAL]] command failed");
    }

    #[test]
    fn refusal_metadata_survives_clone_and_stays_out_of_serialized_content() {
        let result = ToolResult::refusal("sandbox: write denied");
        let cloned = result.clone();
        assert!(cloned.is_guard_refusal());
        assert_eq!(cloned.content, "Error: sandbox: write denied");
        let serialized = serde_json::to_value(&result).unwrap();
        assert_eq!(serialized["content"], "Error: sandbox: write denied");
        assert!(serialized.get("guard_refusal").is_none());
    }
}
