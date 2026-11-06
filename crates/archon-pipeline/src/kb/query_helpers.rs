use super::AnswerContext;

pub(super) fn format_context(context: &AnswerContext) -> String {
    let mut parts = Vec::new();
    for chunk in &context.primary {
        parts.push(format!(
            "### [{}] {} (relevance: {:.2})\n{}",
            chunk.chunk_id, chunk.source_path, chunk.score, chunk.content
        ));
    }
    if !context.summaries.is_empty() {
        parts.push("\n### Compiled summaries:".to_string());
        parts.extend(context.summaries.iter().cloned());
    }
    if !context.concepts.is_empty() {
        parts.push("\n### Related concepts:".to_string());
        parts.extend(context.concepts.iter().cloned());
    }
    parts.join("\n\n")
}

pub(super) fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let prefix: String = text.chars().take(max).collect();
    format!("{prefix}...")
}
