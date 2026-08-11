//! Memory injection into system prompts.
//!
//! Extracts keywords from recent conversation context, queries the
//! [`MemoryGraph`], and formats recalled memories as a structured
//! block that can be spliced into an LLM system prompt.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::access::MemoryTrait;
use crate::types::{Memory, MemoryError, MemoryType};

mod observer;

pub use observer::{
    InjectionObserver, InjectionOutcome, clear_injection_observer, has_injection_observer,
    set_injection_observer,
};

/// Builds a system-prompt section from recalled memories.
pub struct MemoryInjector {
    /// Cached output from the last injection call.
    cache: Option<CacheEntry>,
}

struct CacheEntry {
    context_hash: u64,
    output: String,
    /// The memories this block was built from, kept so a cache hit can report
    /// the same injection a fresh call would.
    ///
    /// A cached block still enters the prompt. Reporting only uncached calls
    /// would make a memory that is used on every turn of a long conversation
    /// read as used once, which is the opposite of what a reuse rate is for.
    recalled: Vec<Memory>,
    /// How many of `recalled` fitted the budget. Always a prefix, because the
    /// formatter stops at the first line that does not fit.
    injected_count: usize,
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
            entry.report(true);
            return Ok(entry.output.clone());
        }

        let keywords = extract_keywords(context);
        if keywords.is_empty() {
            self.cache = Some(CacheEntry::empty(ctx_hash));
            return Ok(String::new());
        }

        let query = keywords.join(" ");
        let memories = graph.recall_memories(&query, DEFAULT_RECALL_LIMIT)?;

        if memories.is_empty() {
            self.cache = Some(CacheEntry::empty(ctx_hash));
            return Ok(String::new());
        }

        let (output, injected_count) = format_memories(&memories, budget_tokens);

        let entry = CacheEntry {
            context_hash: ctx_hash,
            output,
            recalled: memories,
            injected_count,
        };
        // Reported after the block is built and before it is handed back, so an
        // observer sees exactly what the caller is about to receive.
        entry.report(false);
        let output = entry.output.clone();
        self.cache = Some(entry);
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

impl CacheEntry {
    /// An injection that produced nothing: no keywords, or no recall hits.
    fn empty(context_hash: u64) -> Self {
        Self {
            context_hash,
            output: String::new(),
            recalled: Vec::new(),
            injected_count: 0,
        }
    }

    /// Hand this injection to the process-wide observer, if one is installed.
    ///
    /// An empty injection reports nothing. There is no observation to make: no
    /// memory was considered, so no memory can be said to have been passed over.
    /// Reporting it would put rows in the denominator for turns where recall
    /// never ran.
    fn report(&self, from_cache: bool) {
        if self.recalled.is_empty() || !observer::has_injection_observer() {
            return;
        }
        observer::notify(&InjectionOutcome {
            context_hash: self.context_hash,
            recalled: &self.recalled,
            injected: &self.recalled[..self.injected_count],
            from_cache,
        });
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
///
/// Returns the block and how many memories reached it. That count is always a
/// PREFIX length, because the loop stops at the first line that does not fit
/// rather than skipping it and trying the next — so an observer can reconstruct
/// exactly which memories were injected from the ranked input.
fn format_memories(memories: &[Memory], budget_tokens: usize) -> (String, usize) {
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
        return (String::new(), 0);
    }

    let injected_count = lines.len();
    let mut out = String::with_capacity(total_chars);
    out.push_str(header);
    for line in &lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(footer);
    (out, injected_count)
}

fn hash_context(context: &[String]) -> u64 {
    let mut hasher = DefaultHasher::new();
    context.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
#[path = "injection/tests.rs"]
mod tests;
