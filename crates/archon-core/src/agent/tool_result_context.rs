#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContextToolOutput {
    pub content: String,
    pub original_chars: usize,
    pub stored_chars: usize,
    pub limit_chars: usize,
    pub original_bytes: usize,
    pub stored_bytes: usize,
    pub limit_bytes: usize,
    pub truncated: bool,
}

pub(crate) const DEFAULT_MAX_TOOL_RESULT_BYTES: usize = 1_000_000;
pub(crate) const MIN_MAX_TOOL_RESULT_BYTES: usize = 256;

pub(crate) fn resolved_max_tool_result_bytes(
    configured_max: usize,
    provider: &dyn archon_llm::provider::LlmProvider,
) -> usize {
    resolve_max_tool_result_bytes(
        configured_max,
        provider.compaction_policy().max_tool_result_bytes,
    )
}

fn resolve_max_tool_result_bytes(configured_max: usize, provider_max: Option<usize>) -> usize {
    provider_max.map_or(configured_max, |limit| configured_max.min(limit))
}
const DEFAULT_TOOL_RESULT_CONTEXT_BYTES: usize = 64_000;
const SHELL_TOOL_RESULT_CONTEXT_BYTES: usize = 24_000;
const SUBAGENT_TOOL_RESULT_CONTEXT_BYTES: usize = 32_000;

/// Hard ceiling for a tool result when it is first recorded.
///
/// Replay budgets below are intentionally smaller: they manage accumulated
/// context, while this ceiling only blocks pathological single results. The
/// 1 MB default stays below known provider field limits without truncating
/// legitimate built-in tool output (currently capped near 100 KB).
pub(crate) fn emergency_tool_result_bytes(configured_max: usize) -> usize {
    configured_max.min(DEFAULT_TOOL_RESULT_CONTEXT_BYTES)
}

/// Cap a tool result at the moment it is recorded.
///
/// Separate entry point from `cap_tool_output_for_context` so the ingest
/// ceiling and the replay budget cannot drift into each other by accident.
pub(crate) fn cap_tool_output_for_context(tool_name: &str, content: &str) -> ContextToolOutput {
    cap_tool_output_to_bytes(content, context_limit_for_tool(tool_name))
}

pub(crate) fn cap_tool_output_to_bytes(content: &str, limit_bytes: usize) -> ContextToolOutput {
    let original_chars = content.chars().count();
    let original_bytes = content.len();
    let serialized_bytes = serialized_string_bytes(content);
    if serialized_bytes <= limit_bytes {
        return ContextToolOutput {
            content: content.to_string(),
            original_chars,
            stored_chars: original_chars,
            limit_chars: limit_bytes,
            original_bytes,
            stored_bytes: original_bytes,
            limit_bytes,
            truncated: false,
        };
    }

    let mut low = 0;
    let mut high = original_chars;
    while low < high {
        let retained = low + (high - low).div_ceil(2);
        let candidate = capped_content(content, retained);
        if serialized_string_bytes(&candidate) <= limit_bytes {
            low = retained;
        } else {
            high = retained - 1;
        }
    }

    let content = capped_content(content, low);
    let content = if serialized_string_bytes(&content) > limit_bytes {
        truncate_utf8_to_serialized_limit(&content, limit_bytes)
    } else {
        content
    };
    let stored_chars = content.chars().count();
    let stored_bytes = content.len();
    ContextToolOutput {
        content,
        original_chars,
        stored_chars,
        limit_chars: limit_bytes,
        original_bytes,
        stored_bytes,
        limit_bytes,
        truncated: true,
    }
}

fn serialized_string_bytes(content: &str) -> usize {
    serde_json::to_vec(&serde_json::Value::String(content.to_string()))
        .expect("serializing a string cannot fail")
        .len()
}

fn capped_content(content: &str, retained_chars: usize) -> String {
    let head_chars = retained_chars / 2;
    let tail_chars = retained_chars - head_chars;
    let head: String = content.chars().take(head_chars).collect();
    let mut tail: Vec<char> = content.chars().rev().take(tail_chars).collect();
    tail.reverse();
    let tail: String = tail.into_iter().collect();
    let omitted_bytes = content.len().saturating_sub(head.len() + tail.len());
    let marker = format!(
        "\n\n[Archon context note: tool output trimmed; retained {} head bytes and {} tail bytes; omitted {omitted_bytes} bytes before replaying tool output to the model.]\n\n",
        head.len(),
        tail.len(),
    );
    format!("{head}{marker}{tail}")
}

fn truncate_utf8_to_serialized_limit(content: &str, limit_bytes: usize) -> String {
    let mut output = String::new();
    for ch in content.chars() {
        output.push(ch);
        if serialized_string_bytes(&output) > limit_bytes {
            output.pop();
            break;
        }
    }
    output
}

/// The request-boundary projections (#75 A1), split out to keep this file
/// under the size gate.
#[path = "tool_result_projection.rs"]
mod projection;

pub(crate) use projection::{project_messages_for_emergency_retry, project_messages_for_request};

/// Would this result be trimmed when the request projection replays it?
///
/// The ingest ceiling (1 MB) and the per-tool replay budget (24-64 KB) are
/// different numbers, and it is the replay budget that decides what the model
/// actually loses. Spilling is keyed to this, not to the ingest ceiling, or a
/// 30 KB shell result — trimmed on every request — would never get a file.
pub(crate) fn exceeds_context_budget(tool_name: &str, content: &str) -> bool {
    serialized_string_bytes(content) > context_limit_for_tool(tool_name)
}

fn context_limit_for_tool(tool_name: &str) -> usize {
    match tool_name {
        // `TerminalRead` returns shell output and belongs on the shell budget,
        // not the 64 KB default it would otherwise fall through to (#189
        // Phase 6). Over the budget it spills like any other result, so the
        // omitted region stays readable.
        "Bash" | "Shell" | "TerminalRead" => SHELL_TOOL_RESULT_CONTEXT_BYTES,
        "Agent" | "SendMessage" | "TaskCreate" | "TaskOutput" => SUBAGENT_TOOL_RESULT_CONTEXT_BYTES,
        _ => DEFAULT_TOOL_RESULT_CONTEXT_BYTES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_limit_is_capped_by_known_provider_policy() {
        assert_eq!(
            resolve_max_tool_result_bytes(1_000_000, Some(256_000)),
            256_000
        );
        assert_eq!(
            resolve_max_tool_result_bytes(128_000, Some(256_000)),
            128_000
        );
    }

    #[test]
    fn configured_limit_is_used_when_provider_has_no_known_limit() {
        assert_eq!(resolve_max_tool_result_bytes(320_000, None), 320_000);
    }

    #[test]
    fn byte_cap_preserves_utf8_head_tail_and_reports_omitted_bytes() {
        let content = format!("HEAD{}TAIL", "é".repeat(200));
        let output = cap_tool_output_to_bytes(&content, 256);

        assert!(output.truncated);
        assert!(output.content.len() <= 256);
        assert!(output.content.starts_with("HEAD"));
        assert!(output.content.ends_with("TAIL"));
        let (head, marker_and_tail) = output
            .content
            .split_once("\n\n[Archon context note: tool output trimmed; ")
            .expect("visible truncation marker");
        let (marker, tail) = marker_and_tail
            .split_once("]\n\n")
            .expect("marker terminator");
        let omitted_bytes = content.len() - head.len() - tail.len();
        assert_eq!(
            marker,
            format!(
                "retained {} head bytes and {} tail bytes; omitted {omitted_bytes} bytes before replaying tool output to the model.",
                head.len(),
                tail.len(),
            )
        );
        assert_eq!(output.original_bytes, content.len());
        assert_eq!(output.stored_bytes, output.content.len());
        assert_eq!(output.limit_bytes, 256);
    }

    #[test]
    fn byte_cap_is_exact_at_limit_and_truncates_one_byte_over() {
        let exact = "x".repeat(126);
        let exact_output = cap_tool_output_to_bytes(&exact, 128);
        assert!(!exact_output.truncated);
        assert_eq!(exact_output.content, exact);

        let over = format!("{}y", "x".repeat(126));
        let over_output = cap_tool_output_to_bytes(&over, 128);
        assert!(over_output.truncated);
        assert!(over_output.content.len() <= 128);
    }

    #[test]
    fn byte_cap_accounts_for_final_serialized_provider_field() {
        let content = "\\\"\n".repeat(100);
        assert!(content.len() < 512);

        let output = cap_tool_output_to_bytes(&content, 512);
        let serialized = serde_json::to_vec(&serde_json::Value::String(output.content.clone()))
            .expect("serialize provider field");

        assert!(output.truncated);
        assert!(
            serialized.len() <= 512,
            "serialized bytes={}",
            serialized.len()
        );
    }

    #[test]
    fn byte_cap_handles_tiny_limits_without_exceeding_them() {
        for limit in 0..32 {
            let output = cap_tool_output_to_bytes("éééééééé", limit);
            assert!(output.content.len() <= limit, "limit={limit}");
            assert!(std::str::from_utf8(output.content.as_bytes()).is_ok());
        }
    }

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
        assert_eq!(output.limit_chars, SUBAGENT_TOOL_RESULT_CONTEXT_BYTES);
        assert!(output.stored_bytes <= SUBAGENT_TOOL_RESULT_CONTEXT_BYTES);
        assert!(output.content.contains("tool output"));
        assert!(output.content.starts_with('a'));
        assert!(output.content.ends_with('z'));
    }

    #[test]
    fn giant_shell_output_gets_tighter_context_cap() {
        let content = format!("{}{}", "h".repeat(100_000), "t".repeat(100_000));
        let output = cap_tool_output_for_context("Bash", &content);

        assert!(output.truncated);
        assert_eq!(output.limit_chars, SHELL_TOOL_RESULT_CONTEXT_BYTES);
        assert!(output.stored_bytes <= SHELL_TOOL_RESULT_CONTEXT_BYTES);
        assert!(output.content.contains("tool output"));
        assert!(output.content.starts_with('h'));
        assert!(output.content.ends_with('t'));
    }
}
