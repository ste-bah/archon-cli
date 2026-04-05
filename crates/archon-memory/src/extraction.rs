//! Auto-memory extraction from conversation turns.
//!
//! Periodically analyses conversation history and extracts
//! facts, decisions, corrections, patterns, preferences, and rules
//! that should be persisted in the [`MemoryGraph`].

use serde::{Deserialize, Serialize};

use crate::graph::MemoryGraph;
use crate::types::{MemoryError, MemoryType, SearchFilter};

// ── configuration ────────────────────────────────────────────

/// Knobs that control *when* extraction fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionConfig {
    /// How many conversation turns between extraction attempts.
    pub interval: usize,
    /// Master switch.
    pub enabled: bool,
    /// Minimum turns that must elapse after the last extraction
    /// before another one is allowed.
    pub min_turns_between: usize,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            interval: 5,
            enabled: true,
            min_turns_between: 1,
        }
    }
}

// ── state ────────────────────────────────────────────────────

/// Tracks where we are in the conversation so we know when to
/// trigger the next extraction.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExtractionState {
    /// Number of turns since the last successful extraction.
    pub turns_since_last_extraction: usize,
    /// The turn number at which the last extraction happened.
    pub last_extraction_turn: usize,
}

impl ExtractionState {
    /// Record that a turn happened (call once per turn).
    pub fn record_turn(&mut self) {
        self.turns_since_last_extraction += 1;
    }

    /// Record that an extraction just completed at `current_turn`.
    pub fn record_extraction(&mut self, current_turn: usize) {
        self.turns_since_last_extraction = 0;
        self.last_extraction_turn = current_turn;
    }
}

// ── extracted memory ─────────────────────────────────────────

/// A single memory extracted from conversation text, ready to be
/// stored in the graph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractedMemory {
    pub content: String,
    pub memory_type: MemoryType,
    pub tags: Vec<String>,
}

// ── core functions ───────────────────────────────────────────

/// Decide whether it is time to run extraction.
pub fn should_extract(
    config: &ExtractionConfig,
    state: &ExtractionState,
    current_turn: usize,
) -> bool {
    if !config.enabled {
        return false;
    }
    if state.turns_since_last_extraction < config.interval {
        return false;
    }
    let elapsed_since_last = current_turn.saturating_sub(state.last_extraction_turn);
    elapsed_since_last >= config.min_turns_between
}

/// Build the prompt that asks an LLM to extract memories from the
/// given conversation messages.
pub fn build_extraction_prompt(messages: &[String]) -> String {
    let conversation = messages.join("\n---\n");
    format!(
        r#"Analyse the following conversation and extract any important memories.

For each memory, return a JSON object with:
- "content": a concise statement of the fact/decision/preference
- "memory_type": one of "fact", "decision", "correction", "pattern", "preference", "rule"
- "tags": a list of short keyword tags

Return a JSON array of these objects. If there is nothing worth remembering, return an empty array `[]`.

Conversation:
{conversation}
"#
    )
}

/// Parse the JSON response from the LLM into [`ExtractedMemory`] values.
///
/// Returns `Ok(vec![])` rather than an error when the input is
/// not valid JSON or contains no extractable items — callers should
/// not crash on bad LLM output.
pub fn parse_extraction_response(json_str: &str) -> Result<Vec<ExtractedMemory>, MemoryError> {
    // Try to parse the whole string as an array first.
    let items: Vec<RawExtracted> = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => {
            // Maybe the LLM wrapped it in markdown fences — try stripping.
            let stripped = json_str
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim();
            match serde_json::from_str(stripped) {
                Ok(v) => v,
                // Graceful degradation: return empty vec, do not crash.
                Err(_) => return Ok(Vec::new()),
            }
        }
    };

    let mut out = Vec::with_capacity(items.len());
    for raw in items {
        let memory_type = match MemoryType::from_str_opt(&raw.memory_type) {
            Some(mt) => mt,
            None => continue, // skip unknown types
        };
        if raw.content.trim().is_empty() {
            continue;
        }
        out.push(ExtractedMemory {
            content: raw.content,
            memory_type,
            tags: raw.tags,
        });
    }
    Ok(out)
}

/// Store extracted memories in the graph, skipping duplicates.
///
/// A memory is considered a duplicate when an existing memory's
/// content contains the new content as a substring (case-insensitive)
/// or vice-versa.
///
/// Returns the number of memories that were actually stored.
pub fn store_extracted(
    graph: &MemoryGraph,
    memories: &[ExtractedMemory],
    session_id: &str,
) -> Result<usize, MemoryError> {
    let mut stored = 0usize;

    for mem in memories {
        // Dedup: check for similar content already in the graph.
        let filter = SearchFilter {
            text: Some(mem.content.clone()),
            ..Default::default()
        };
        let existing = graph.search_memories(&filter)?;
        let dominated = existing.iter().any(|e| {
            let lc_existing = e.content.to_lowercase();
            let lc_new = mem.content.to_lowercase();
            lc_existing.contains(&lc_new) || lc_new.contains(&lc_existing)
        });
        if dominated {
            continue;
        }

        let mut tags = mem.tags.clone();
        tags.push("auto-extract".into());
        tags.push(format!("session:{session_id}"));

        graph.store_memory(
            &mem.content,
            "", // title
            mem.memory_type,
            0.5, // default importance
            &tags,
            "auto-extract",
            "", // project_path
        )?;
        stored += 1;
    }

    Ok(stored)
}

// ── internal helpers ─────────────────────────────────────────

/// Intermediate serde target that mirrors the JSON the LLM produces.
#[derive(Deserialize)]
struct RawExtracted {
    content: String,
    memory_type: String,
    #[serde(default)]
    tags: Vec<String>,
}

// ── tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- should_extract -------------------------------------------------

    #[test]
    fn should_extract_fires_at_interval() {
        let config = ExtractionConfig {
            interval: 5,
            enabled: true,
            min_turns_between: 1,
        };
        let mut state = ExtractionState::default();
        // Simulate 4 turns — not yet.
        for _ in 0..4 {
            state.record_turn();
        }
        assert!(!should_extract(&config, &state, 4));

        // 5th turn — should fire.
        state.record_turn();
        assert!(should_extract(&config, &state, 5));
    }

    #[test]
    fn should_extract_respects_min_turns_between() {
        let config = ExtractionConfig {
            interval: 1,
            enabled: true,
            min_turns_between: 3,
        };
        let mut state = ExtractionState::default();
        state.record_turn();
        // Last extraction was at turn 0, current turn is 1 — only 1 elapsed.
        assert!(!should_extract(&config, &state, 1));

        // current_turn = 3 — 3 turns since last_extraction_turn 0
        assert!(should_extract(&config, &state, 3));
    }

    #[test]
    fn should_extract_disabled() {
        let config = ExtractionConfig {
            interval: 1,
            enabled: false,
            min_turns_between: 0,
        };
        let mut state = ExtractionState::default();
        for _ in 0..10 {
            state.record_turn();
        }
        assert!(!should_extract(&config, &state, 10));
    }

    // -- parse_extraction_response --------------------------------------

    #[test]
    fn parse_valid_json() {
        let json = r#"[
            {"content": "User prefers dark mode", "memory_type": "preference", "tags": ["ui"]},
            {"content": "Project uses Rust 2024 edition", "memory_type": "fact", "tags": ["rust", "config"]}
        ]"#;
        let mems = parse_extraction_response(json).expect("should parse");
        assert_eq!(mems.len(), 2);
        assert_eq!(mems[0].memory_type, MemoryType::Preference);
        assert_eq!(mems[0].tags, vec!["ui"]);
        assert_eq!(mems[1].memory_type, MemoryType::Fact);
    }

    #[test]
    fn parse_invalid_json_no_crash() {
        let bad = "this is not json at all {{{}}}";
        let mems = parse_extraction_response(bad).expect("should not error");
        assert!(mems.is_empty());
    }

    #[test]
    fn parse_markdown_fenced_json() {
        let fenced = r#"```json
[{"content":"a rule","memory_type":"rule","tags":[]}]
```"#;
        let mems = parse_extraction_response(fenced).expect("should parse");
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].memory_type, MemoryType::Rule);
    }

    #[test]
    fn parse_skips_unknown_types_and_empty_content() {
        let json = r#"[
            {"content":"good","memory_type":"fact","tags":[]},
            {"content":"","memory_type":"fact","tags":[]},
            {"content":"alien","memory_type":"unknown_type","tags":[]}
        ]"#;
        let mems = parse_extraction_response(json).expect("should parse");
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].content, "good");
    }

    // -- build_extraction_prompt ----------------------------------------

    #[test]
    fn build_prompt_includes_messages() {
        let msgs = vec!["Hello".to_string(), "How are you?".to_string()];
        let prompt = build_extraction_prompt(&msgs);
        assert!(prompt.contains("Hello"));
        assert!(prompt.contains("How are you?"));
        assert!(prompt.contains("JSON"));
    }

    // -- store_extracted ------------------------------------------------

    #[test]
    fn store_with_tags() {
        let graph = MemoryGraph::in_memory().expect("in-memory graph");
        let mems = vec![ExtractedMemory {
            content: "Rust edition is 2024".into(),
            memory_type: MemoryType::Fact,
            tags: vec!["rust".into()],
        }];
        let stored = store_extracted(&graph, &mems, "sess-001").expect("store");
        assert_eq!(stored, 1);

        let results = graph.recall_memories("Rust edition", 10).expect("recall");
        assert_eq!(results.len(), 1);
        assert!(results[0].tags.contains(&"auto-extract".to_string()));
        assert!(results[0].tags.contains(&"session:sess-001".to_string()));
        assert!(results[0].tags.contains(&"rust".to_string()));
        assert_eq!(results[0].source_type, "auto-extract");
    }

    #[test]
    fn dedup_skips_substring_match() {
        let graph = MemoryGraph::in_memory().expect("in-memory graph");

        // Pre-populate with an existing memory.
        graph
            .store_memory(
                "User prefers dark mode in all editors",
                "",
                MemoryType::Preference,
                0.5,
                &[],
                "manual",
                "",
            )
            .expect("seed");

        // Try to store a substring of the existing memory.
        let mems = vec![ExtractedMemory {
            content: "dark mode in all editors".into(),
            memory_type: MemoryType::Preference,
            tags: vec![],
        }];
        let stored = store_extracted(&graph, &mems, "s1").expect("store");
        assert_eq!(stored, 0, "duplicate should be skipped");
    }

    // -- extraction state tracking --------------------------------------

    #[test]
    fn extraction_state_tracking() {
        let mut state = ExtractionState::default();
        assert_eq!(state.turns_since_last_extraction, 0);
        assert_eq!(state.last_extraction_turn, 0);

        state.record_turn();
        state.record_turn();
        assert_eq!(state.turns_since_last_extraction, 2);

        state.record_extraction(7);
        assert_eq!(state.turns_since_last_extraction, 0);
        assert_eq!(state.last_extraction_turn, 7);

        state.record_turn();
        assert_eq!(state.turns_since_last_extraction, 1);
    }
}
