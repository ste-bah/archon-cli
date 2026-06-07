//! Query normalization helpers for document retrieval.

const MIN_TERM_LEN: usize = 2;

const STOPWORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "by", "did", "do", "does", "explain",
    "for", "from", "how", "in", "is", "it", "know", "me", "of", "on", "or", "tell", "the", "to",
    "was", "what", "when", "where", "who", "why", "with", "you", "your",
];

/// Convert a user question into Cozo FTS-safe terms.
///
/// Cozo's FTS parser treats punctuation as syntax. A normal question such as
/// `What is X?` can otherwise fail before retrieval starts.
pub fn safe_fts_query(query: &str) -> String {
    let cleaned = strip_outer_quotes(query)
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>();
    cleaned
        .split_whitespace()
        .filter(|term| term.len() >= MIN_TERM_LEN)
        .filter(|term| !STOPWORDS.contains(term))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn normalized_exact_text(query: &str) -> String {
    strip_outer_quotes(query).trim().to_string()
}

fn strip_outer_quotes(query: &str) -> &str {
    let trimmed = query.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| {
            trimmed
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
        })
        .unwrap_or(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_query_strips_question_punctuation() {
        let query = "What is the Traders Reality Hybrid System?";
        assert_eq!(safe_fts_query(query), "traders reality hybrid system");
    }

    #[test]
    fn fts_query_preserves_specific_version_terms() {
        let query = "Magix Sequoia 14/15/16 vs Boris-FX Sequoia 2026";
        assert_eq!(
            safe_fts_query(query),
            "magix sequoia 14 15 16 vs boris-fx sequoia 2026"
        );
    }

    #[test]
    fn fts_query_removes_conversational_words() {
        let query = "What do you know about the hybrid systm?";
        assert_eq!(safe_fts_query(query), "hybrid systm");
    }

    #[test]
    fn exact_text_only_strips_wrapping_quotes() {
        assert_eq!(normalized_exact_text("\"blue falcon\""), "blue falcon");
    }
}
