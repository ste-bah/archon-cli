#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextToolOutput {
    pub content: String,
    pub original_chars: usize,
    pub stored_chars: usize,
    pub limit_chars: usize,
    pub truncated: bool,
}

const DEFAULT_TOOL_RESULT_CONTEXT_CHARS: usize = 64_000;
const SUBAGENT_TOOL_RESULT_CONTEXT_CHARS: usize = 32_000;

pub(crate) fn cap_tool_output_for_context(tool_name: &str, content: &str) -> ContextToolOutput {
    let limit_chars = context_limit_for_tool(tool_name);
    let original_chars = content.chars().count();
    if original_chars <= limit_chars {
        return ContextToolOutput {
            content: content.to_string(),
            original_chars,
            stored_chars: original_chars,
            limit_chars,
            truncated: false,
        };
    }

    let marker = format!(
        "\n\n[Archon context note: tool output trimmed from {original_chars} chars before replaying it to the model. Full output was emitted to UI/logs.]\n\n"
    );
    let marker_chars = marker.chars().count();
    let body_budget = limit_chars.saturating_sub(marker_chars).max(1);
    let head_chars = body_budget / 2;
    let tail_chars = body_budget.saturating_sub(head_chars);

    let head: String = content.chars().take(head_chars).collect();
    let mut tail_vec: Vec<char> = content.chars().rev().take(tail_chars).collect();
    tail_vec.reverse();
    let tail: String = tail_vec.into_iter().collect();
    let trimmed = format!("{head}{marker}{tail}");
    let stored_chars = trimmed.chars().count();

    ContextToolOutput {
        content: trimmed,
        original_chars,
        stored_chars,
        limit_chars,
        truncated: true,
    }
}

fn context_limit_for_tool(tool_name: &str) -> usize {
    match tool_name {
        "Agent" | "SendMessage" | "TaskCreate" | "TaskOutput" => SUBAGENT_TOOL_RESULT_CONTEXT_CHARS,
        _ => DEFAULT_TOOL_RESULT_CONTEXT_CHARS,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_tool_output_is_left_unchanged() {
        let output = cap_tool_output_for_context("Read", "small");

        assert!(!output.truncated);
        assert_eq!(output.content, "small");
        assert_eq!(output.original_chars, 5);
        assert_eq!(output.stored_chars, 5);
    }

    #[test]
    fn large_subagent_output_is_trimmed_for_context() {
        let content = format!("{}{}", "a".repeat(40_000), "z".repeat(40_000));
        let output = cap_tool_output_for_context("Agent", &content);

        assert!(output.truncated);
        assert_eq!(output.limit_chars, SUBAGENT_TOOL_RESULT_CONTEXT_CHARS);
        assert!(output.stored_chars <= SUBAGENT_TOOL_RESULT_CONTEXT_CHARS);
        assert!(output.content.contains("tool output trimmed"));
        assert!(output.content.starts_with('a'));
        assert!(output.content.ends_with('z'));
    }
}
