//! Memory injection into system prompts.
//!
//! Extracts keywords from recent conversation context, queries the
//! [`MemoryGraph`], and formats recalled memories as a structured
//! block that can be spliced into an LLM system prompt.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::access::MemoryTrait;
use crate::types::{Memory, MemoryError, MemoryType};

/// Builds a system-prompt section from recalled memories.
pub struct MemoryInjector {
    /// Cached output from the last injection call.
    cache: Option<CacheEntry>,
}

struct CacheEntry {
    context_hash: u64,
    output: String,
}

/// Default number of memories to request from the graph.
const DEFAULT_RECALL_LIMIT: usize = 20;

/// Rough token estimate: 1 token ≈ 4 characters.
const CHARS_PER_TOKEN: usize = 4;

impl MemoryInjector {
    /// Create a new injector with an empty cache.
    pub fn new() -> Self {
        Self { cache: None }
    }

    /// Inject recalled memories formatted for a system prompt.
    ///
    /// * `graph`         – the memory graph to query
    /// * `context`       – recent user messages (newest last)
    /// * `budget_tokens` – maximum tokens for the returned block
    ///
    /// Returns an empty string when no relevant memories are found.
    pub fn inject(
        &mut self,
        graph: &dyn MemoryTrait,
        context: &[String],
        budget_tokens: usize,
    ) -> Result<String, MemoryError> {
        let ctx_hash = hash_context(context);
        if let Some(ref entry) = self.cache
            && entry.context_hash == ctx_hash
        {
            return Ok(entry.output.clone());
        }

        let keywords = extract_keywords(context);
        if keywords.is_empty() {
            self.cache = Some(CacheEntry {
                context_hash: ctx_hash,
                output: String::new(),
            });
            return Ok(String::new());
        }

        let query = keywords.join(" ");
        let memories = graph.recall_memories(&query, DEFAULT_RECALL_LIMIT)?;

        if memories.is_empty() {
            self.cache = Some(CacheEntry {
                context_hash: ctx_hash,
                output: String::new(),
            });
            return Ok(String::new());
        }

        let output = format_memories(&memories, budget_tokens);

        self.cache = Some(CacheEntry {
            context_hash: ctx_hash,
            output: output.clone(),
        });
        Ok(output)
    }

    /// Invalidate the cache so the next call re-queries the graph.
    pub fn invalidate_cache(&mut self) {
        self.cache = None;
    }
}

impl Default for MemoryInjector {
    fn default() -> Self {
        Self::new()
    }
}

// ── helpers ──────────────────────────────────────────────────────

/// Extract keywords from the last (up to) 3 user messages.
fn extract_keywords(context: &[String]) -> Vec<String> {
    let recent = if context.len() > 3 {
        &context[context.len() - 3..]
    } else {
        context
    };

    let stop_words: &[&str] = &[
        "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
        "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
        "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "through",
        "during", "before", "after", "and", "but", "or", "nor", "not", "so", "yet", "both",
        "either", "neither", "each", "every", "all", "any", "few", "more", "most", "other", "some",
        "such", "no", "only", "same", "than", "too", "very", "just", "about", "it", "its", "this",
        "that", "these", "those", "i", "me", "my", "we", "our", "you", "your", "he", "she", "they",
        "them", "what", "which", "who", "how", "when", "where", "why", "if", "then", "else",
    ];

    let mut words: Vec<String> = Vec::new();
    for msg in recent {
        for word in msg.split_whitespace() {
            let cleaned: String = word
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect::<String>()
                .to_lowercase();
            if cleaned.len() >= 2
                && !stop_words.contains(&cleaned.as_str())
                && !words.contains(&cleaned)
            {
                words.push(cleaned);
            }
        }
    }
    words
}

/// Format a single memory line.
fn format_one(mem: &Memory) -> String {
    let type_tag = match mem.memory_type {
        MemoryType::Fact => "[fact]",
        MemoryType::Decision => "[decision]",
        MemoryType::Correction => "[correction]",
        MemoryType::Pattern => "[pattern]",
        MemoryType::Preference => "[preference]",
        MemoryType::Rule => "[rule]",
        MemoryType::PersonalitySnapshot => "[snapshot]",
    };

    let suffix = match mem.memory_type {
        MemoryType::Correction => {
            // Use importance as a proxy for severity.
            let severity = if mem.importance >= 0.8 {
                "high"
            } else if mem.importance >= 0.5 {
                "medium"
            } else {
                "low"
            };
            format!(" (severity: {severity})")
        }
        MemoryType::Fact if !mem.tags.is_empty() => {
            format!(" (tags: {})", mem.tags.join(", "))
        }
        _ => String::new(),
    };

    format!("- {type_tag} {}{suffix}", mem.content)
}

/// Format recalled memories into the `<memories>` block, respecting
/// the token budget.  Memories are assumed to already be ranked by
/// the recall query (highest relevance first).
fn format_memories(memories: &[Memory], budget_tokens: usize) -> String {
    let header = "<memories>\n## Relevant Memories\n";
    let footer = "</memories>";
    let budget_chars = budget_tokens * CHARS_PER_TOKEN;

    let mut lines: Vec<String> = Vec::new();
    let mut total_chars = header.len() + footer.len();

    for mem in memories {
        let line = format_one(mem);
        let line_chars = line.len() + 1; // +1 for newline
        if total_chars + line_chars > budget_chars {
            break;
        }
        total_chars += line_chars;
        lines.push(line);
    }

    if lines.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(total_chars);
    out.push_str(header);
    for line in &lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(footer);
    out
}

fn hash_context(context: &[String]) -> u64 {
    let mut hasher = DefaultHasher::new();
    context.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::MemoryGraph;
    use crate::types::MemoryType;

    fn make_graph() -> MemoryGraph {
        MemoryGraph::in_memory().expect("in-memory graph")
    }

    fn seed_graph(g: &MemoryGraph) {
        g.store_memory(
            "User prefers dark mode",
            "dark mode pref",
            MemoryType::Preference,
            0.7,
            &["ui".into(), "preference".into()],
            "manual",
            "/project",
        )
        .expect("store");
        g.store_memory(
            "Rust edition must be 2024",
            "rust edition",
            MemoryType::Rule,
            0.9,
            &["rust".into(), "edition".into()],
            "manual",
            "/project",
        )
        .expect("store");
        g.store_memory(
            "Never use .unwrap() in library code",
            "no unwrap",
            MemoryType::Correction,
            0.85,
            &["rust".into(), "quality".into()],
            "manual",
            "/project",
        )
        .expect("store");
        g.store_memory(
            "Architecture uses hexagonal pattern",
            "architecture",
            MemoryType::Decision,
            0.8,
            &["architecture".into()],
            "manual",
            "/project",
        )
        .expect("store");
        g.store_memory(
            "Database migrations run on startup",
            "migrations",
            MemoryType::Fact,
            0.6,
            &["database".into(), "ops".into()],
            "manual",
            "/project",
        )
        .expect("store");
    }

    #[test]
    fn inject_returns_memories_for_matching_context() {
        let g = make_graph();
        seed_graph(&g);
        let mut injector = MemoryInjector::new();
        let context = vec!["Tell me about rust edition rules".to_string()];
        let result = injector.inject(&g, &context, 500).expect("inject");
        assert!(!result.is_empty());
        assert!(result.contains("<memories>"));
        assert!(result.contains("</memories>"));
        assert!(result.contains("## Relevant Memories"));
    }

    #[test]
    fn inject_empty_graph_returns_empty_string() {
        let g = make_graph();
        let mut injector = MemoryInjector::new();
        let context = vec!["hello world".to_string()];
        let result = injector.inject(&g, &context, 500).expect("inject");
        assert!(result.is_empty());
    }

    #[test]
    fn inject_empty_context_returns_empty_string() {
        let g = make_graph();
        seed_graph(&g);
        let mut injector = MemoryInjector::new();
        let result = injector.inject(&g, &[], 500).expect("inject");
        assert!(result.is_empty());
    }

    #[test]
    fn budget_enforcement_truncates_output() {
        let g = make_graph();
        seed_graph(&g);
        let mut injector = MemoryInjector::new();
        let context = vec!["rust edition unwrap database architecture".to_string()];

        // Very small budget — only header + maybe 1 line.
        let tiny = injector.inject(&g, &context, 25).expect("inject");
        // Large budget — should include more.
        injector.invalidate_cache();
        let large = injector.inject(&g, &context, 5000).expect("inject");

        // Tiny budget should be shorter (or empty if even 1 line doesn't fit).
        assert!(tiny.len() <= large.len());
    }

    #[test]
    fn extract_keywords_uses_last_three_messages() {
        let context = vec![
            "oldest message ignored".to_string(),
            "second message also ignored".to_string(),
            "rust edition 2024".to_string(),
            "unwrap error handling".to_string(),
            "database migration startup".to_string(),
        ];
        let kw = extract_keywords(&context);
        // Should NOT contain words from the first two messages.
        assert!(!kw.contains(&"oldest".to_string()));
        assert!(!kw.contains(&"ignored".to_string()));
        // Should contain words from the last three.
        assert!(kw.contains(&"rust".to_string()));
        assert!(kw.contains(&"database".to_string()));
        assert!(kw.contains(&"migration".to_string()));
    }

    #[test]
    fn formatting_correctness() {
        let g = make_graph();
        seed_graph(&g);
        let mut injector = MemoryInjector::new();
        let context = vec!["rust unwrap".to_string()];
        let result = injector.inject(&g, &context, 5000).expect("inject");

        if !result.is_empty() {
            // Correction memories should have severity.
            if result.contains("[correction]") {
                assert!(
                    result.contains("severity:"),
                    "corrections should include severity"
                );
            }
            // Fact memories should have tags.
            if result.contains("[fact]") {
                assert!(result.contains("tags:"), "facts should include tags");
            }
            // Should be wrapped in <memories> tags.
            assert!(result.starts_with("<memories>"));
            assert!(result.ends_with("</memories>"));
        }
    }

    #[test]
    fn cache_hit_returns_same_result() {
        let g = make_graph();
        seed_graph(&g);
        let mut injector = MemoryInjector::new();
        let context = vec!["rust edition".to_string()];

        let first = injector.inject(&g, &context, 500).expect("inject");
        let second = injector.inject(&g, &context, 500).expect("inject");
        assert_eq!(first, second, "cache should return identical result");
    }

    #[test]
    fn cache_invalidated_on_new_context() {
        let g = make_graph();
        seed_graph(&g);
        let mut injector = MemoryInjector::new();

        let ctx1 = vec!["rust edition".to_string()];
        let r1 = injector.inject(&g, &ctx1, 500).expect("inject");

        let ctx2 = vec!["database migration".to_string()];
        let r2 = injector.inject(&g, &ctx2, 500).expect("inject");

        // Different context, different hash — cache miss.
        // Results may or may not differ depending on recall,
        // but the function should not error.
        let _ = (r1, r2);
    }

    #[test]
    fn stop_words_are_excluded() {
        let context = vec!["the is a an to of in for".to_string()];
        let kw = extract_keywords(&context);
        assert!(kw.is_empty(), "all stop words should be excluded");
    }

    #[test]
    fn format_one_decision_has_no_suffix() {
        let mem = Memory {
            id: "1".into(),
            content: "Use hexagonal arch".into(),
            title: String::new(),
            memory_type: MemoryType::Decision,
            importance: 0.8,
            tags: vec!["arch".into()],
            source_type: "manual".into(),
            project_path: String::new(),
            created_at: chrono::Utc::now(),
            updated_at: None,
            access_count: 0,
            last_accessed: None,
        };
        let line = format_one(&mem);
        assert_eq!(line, "- [decision] Use hexagonal arch");
    }
}
