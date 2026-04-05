use crate::boundary::CompactionStrategy;
use crate::messages::ContextMessage;

/// The summary prompt template used for compaction.
pub const SUMMARY_PROMPT: &str = "\
Summarize this conversation for context continuity. You MUST preserve:
- All file paths mentioned and their current state
- Key decisions made and their rationale
- Current task and progress
- Any errors encountered and their resolution
- Technical details (function names, API endpoints, config values)
Keep the summary under 2000 tokens. Use bullet points. Do not include pleasantries or meta-commentary.";

/// Default number of recent turns (user+assistant pairs) to preserve verbatim.
pub const DEFAULT_PRESERVE_RECENT_TURNS: usize = 3;

/// Compact a conversation by replacing old messages with a summary.
///
/// Preserves the last `preserve_recent` turns (user+assistant pairs).
/// Returns the compacted message list: [summary_message, ...recent_messages].
///
/// The caller is responsible for generating the actual summary text
/// (via an LLM call). This function handles the message list manipulation.
pub fn compact_messages(
    messages: &[ContextMessage],
    summary_text: &str,
    preserve_recent: usize,
) -> Vec<ContextMessage> {
    if messages.len() <= preserve_recent * 2 {
        // Not enough messages to compact
        return messages.to_vec();
    }

    let split_point = messages.len().saturating_sub(preserve_recent * 2);

    let mut compacted = Vec::new();

    // Add structured summary as first user message
    let header = build_structured_summary_header(summary_text);
    compacted.push(ContextMessage::user(&header));

    // Add preserved recent messages (including any tool results)
    compacted.extend_from_slice(&messages[split_point..]);

    compacted
}

/// Compact with the default number of preserved turns ([`DEFAULT_PRESERVE_RECENT_TURNS`]).
pub fn compact_messages_default(
    messages: &[ContextMessage],
    summary_text: &str,
) -> Vec<ContextMessage> {
    compact_messages(messages, summary_text, DEFAULT_PRESERVE_RECENT_TURNS)
}

/// Build a structured summary header from raw summary text.
///
/// Wraps the summary in a well-known format that downstream consumers
/// (e.g. the agent loop) can parse if needed.
pub fn build_structured_summary_header(summary_text: &str) -> String {
    let mut header = String::with_capacity(summary_text.len() + 128);
    header.push_str("[Context Summary]\n");
    header.push_str("## Key Decisions\n");
    header.push_str("## File Changes\n");
    header.push_str("## Current State\n\n");
    header.push_str(summary_text);
    header
}

/// Extract file paths mentioned in the messages being compacted.
///
/// Scans for common path-like patterns to include in the summary header.
/// Returns de-duplicated paths in the order they first appeared.
pub fn extract_mentioned_paths(messages: &[ContextMessage]) -> Vec<String> {
    let mut paths = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for msg in messages {
        let text = match &msg.content {
            serde_json::Value::String(s) => s.as_str(),
            _ => continue,
        };

        for word in text.split_whitespace() {
            // Heuristic: looks like a file path if it contains a slash and a dot,
            // or starts with ./ or /
            let trimmed =
                word.trim_matches(|c: char| c == '`' || c == '"' || c == '\'' || c == ',');
            let looks_like_path = (trimmed.contains('/') && trimmed.contains('.'))
                || trimmed.starts_with("./")
                || trimmed.starts_with('/');
            if looks_like_path && seen.insert(trimmed.to_string()) {
                paths.push(trimmed.to_string());
            }
        }
    }

    paths
}

/// Build the messages to send to the LLM for generating a summary.
///
/// Takes the messages that will be compacted (excluded from preserved recent).
pub fn build_summary_request(
    messages: &[ContextMessage],
    preserve_recent: usize,
) -> Vec<ContextMessage> {
    let split_point = messages.len().saturating_sub(preserve_recent * 2);
    let to_summarize = &messages[..split_point];

    let mut summary_messages = Vec::new();

    // Format the old messages as a single text block
    let mut conversation_text = String::new();
    for msg in to_summarize {
        let content = match &msg.content {
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        conversation_text.push_str(&format!("[{}]: {}\n\n", msg.role, content));
    }

    // Include any detected file paths for the summarizer
    let paths = extract_mentioned_paths(to_summarize);
    let path_hint = if paths.is_empty() {
        String::new()
    } else {
        format!(
            "\n\nFiles mentioned in conversation:\n{}\n",
            paths.join("\n")
        )
    };

    summary_messages.push(ContextMessage::user(&format!(
        "{SUMMARY_PROMPT}{path_hint}\n\n---\n\n{conversation_text}"
    )));

    summary_messages
}

/// Select compaction strategy based on context usage ratio.
///
/// Returns `None` when usage is below 60 % and compaction is not needed.
pub fn select_strategy(usage_ratio: f32) -> Option<CompactionStrategy> {
    if usage_ratio >= 0.90 {
        Some(CompactionStrategy::Snip)
    } else if usage_ratio >= 0.80 {
        Some(CompactionStrategy::Auto)
    } else if usage_ratio >= 0.60 {
        Some(CompactionStrategy::Micro)
    } else {
        None
    }
}

/// Compaction statistics returned after a compaction operation.
#[derive(Debug, Clone)]
pub struct CompactionStats {
    pub strategy: CompactionStrategy,
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub messages_removed: usize,
    pub ratio: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_preserves_recent() {
        let messages: Vec<ContextMessage> = (0..10)
            .map(|i| {
                if i % 2 == 0 {
                    ContextMessage::user(&format!("user msg {i}"))
                } else {
                    ContextMessage::assistant(&format!("assistant msg {i}"))
                }
            })
            .collect();

        let compacted = compact_messages(&messages, "Summary of earlier conversation", 3);

        // Should have: 1 summary + 6 recent (3 pairs)
        assert_eq!(compacted.len(), 7);
        let first_content = compacted[0].content.as_str().expect("string content");
        assert!(first_content.contains("[Context Summary]"));
        assert!(first_content.contains("Summary of earlier conversation"));

        // Last message should be the original last message
        let last = compacted.last().expect("last");
        assert!(
            last.content
                .as_str()
                .map(|s| s.contains("msg 9"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn compact_too_few_messages_returns_unchanged() {
        let messages = vec![
            ContextMessage::user("hello"),
            ContextMessage::assistant("hi"),
        ];

        let compacted = compact_messages(&messages, "summary", 3);
        assert_eq!(compacted.len(), 2); // unchanged
    }

    #[test]
    fn compact_default_uses_three_turns() {
        let messages: Vec<ContextMessage> = (0..12)
            .map(|i| {
                if i % 2 == 0 {
                    ContextMessage::user(&format!("user msg {i}"))
                } else {
                    ContextMessage::assistant(&format!("assistant msg {i}"))
                }
            })
            .collect();

        let compacted = compact_messages_default(&messages, "summary text");
        // 1 summary + 6 recent (3 pairs)
        assert_eq!(compacted.len(), 7);
    }

    #[test]
    fn build_summary_request_contains_old_messages() {
        let messages: Vec<ContextMessage> = (0..8)
            .map(|i| ContextMessage::user(&format!("msg {i}")))
            .collect();

        let request = build_summary_request(&messages, 2);
        assert_eq!(request.len(), 1);

        let content = request[0].content.as_str().expect("string");
        assert!(content.contains("msg 0")); // old message included
        assert!(content.contains("msg 3")); // old message included
        assert!(!content.contains("msg 6")); // recent, excluded
        assert!(!content.contains("msg 7")); // recent, excluded
    }

    #[test]
    fn structured_summary_header_format() {
        let header = build_structured_summary_header("- decided to use Rust");
        assert!(header.starts_with("[Context Summary]"));
        assert!(header.contains("## Key Decisions"));
        assert!(header.contains("## File Changes"));
        assert!(header.contains("## Current State"));
        assert!(header.contains("- decided to use Rust"));
    }

    #[test]
    fn extract_paths_from_messages() {
        let messages = vec![
            ContextMessage::user("I edited `src/main.rs` and ./Cargo.toml"),
            ContextMessage::assistant("Updated /etc/config.yaml too"),
            ContextMessage::user("no paths here"),
        ];

        let paths = extract_mentioned_paths(&messages);
        assert!(paths.contains(&"src/main.rs".to_string()));
        assert!(paths.contains(&"./Cargo.toml".to_string()));
        assert!(paths.contains(&"/etc/config.yaml".to_string()));
        // No duplicates
        assert_eq!(
            paths.len(),
            paths.iter().collect::<std::collections::HashSet<_>>().len()
        );
    }

    #[test]
    fn extract_paths_deduplicates() {
        let messages = vec![ContextMessage::user("src/lib.rs src/lib.rs src/lib.rs")];
        let paths = extract_mentioned_paths(&messages);
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn summary_request_includes_path_hint() {
        let messages = vec![
            ContextMessage::user("edited src/main.rs"),
            ContextMessage::assistant("ok"),
            ContextMessage::user("now what"),
            ContextMessage::assistant("done"),
        ];

        let request = build_summary_request(&messages, 1);
        let content = request[0].content.as_str().expect("string");
        assert!(content.contains("src/main.rs"));
        assert!(content.contains("Files mentioned"));
    }
}
