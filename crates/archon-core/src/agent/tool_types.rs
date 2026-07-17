use std::sync::Arc;

pub(super) fn tool_transcript_summary(
    tool_name: &str,
    input: &serde_json::Value,
) -> Option<String> {
    if tool_name != "Bash" {
        return None;
    }

    let command = input.get("command")?.as_str()?;
    let normalized = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }

    const DISPLAY_LIMIT: usize = 80;
    if normalized.chars().count() <= DISPLAY_LIMIT {
        return Some(normalized);
    }

    let end = normalized
        .char_indices()
        .nth(DISPLAY_LIMIT - 1)
        .map(|(index, _)| index)
        .unwrap_or(normalized.len());
    Some(format!("{}…", &normalized[..end]))
}

pub(super) struct PreflightResult {
    pub(super) tool_name: String,
    pub(super) tool_id: String,
    pub(super) input: serde_json::Value,
    pub(super) tool_arc: Arc<dyn archon_tools::tool::Tool>,
    pub(super) file_path: Option<String>,
    pub(super) sandbox_prechecked: bool,
}

#[cfg(test)]
mod tests {
    use super::tool_transcript_summary;

    #[test]
    fn bash_summary_uses_only_normalized_command_and_caps_display_characters() {
        let input = serde_json::json!({
            "command": "  cargo   test\n  -p archon-tui  ",
            "secret": "must not appear",
        });
        assert_eq!(
            tool_transcript_summary("Bash", &input),
            Some("cargo test -p archon-tui".to_string())
        );

        let long_command = "é".repeat(100);
        let summary = tool_transcript_summary("Bash", &serde_json::json!({ "command": long_command }))
            .expect("Bash command summary");
        assert!(summary.ends_with('…'));
        assert_eq!(summary.chars().count(), 80);
        assert!(!summary.contains("secret"));
    }

    #[test]
    fn non_bash_or_missing_command_has_no_summary() {
        assert_eq!(
            tool_transcript_summary("Read", &serde_json::json!({ "command": "cat secret" })),
            None
        );
        assert_eq!(tool_transcript_summary("Bash", &serde_json::json!({ "args": "x" })), None);
    }
}
