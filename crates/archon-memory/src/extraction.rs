//! Auto-memory extraction from conversation turns.
//!
//! Periodically analyses conversation history and extracts
//! facts, decisions, corrections, patterns, preferences, and rules
//! that should be persisted in the [`MemoryGraph`].

use serde::{Deserialize, Serialize};

use crate::access::MemoryTrait;
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

// ── ingest limits ────────────────────────────────────────────

/// Longest content accepted for a general extracted memory.
///
/// Extraction hands an LLM the whole conversation and stores whatever comes
/// back. With no ceiling, a pasted document becomes a "memory" verbatim --
/// observed as five copies of one PRD and roughly twenty of another. Anything
/// this long is a transcript, not something learned from it.
pub const MAX_EXTRACTED_CONTENT_CHARS: usize = 2_000;

/// Longest content accepted for a [`MemoryType::Rule`].
///
/// Far tighter than the general cap because rules are unconditionally rendered
/// into the system prompt by `RulesEngine::format_for_prompt`, so an oversized
/// one is paid for on every request forever, not just when recalled. A stored
/// operating manual reached the prompt this way and read as a malformed rule.
/// A behavioural rule that does not fit in a couple of sentences is not a rule.
pub const MAX_RULE_CONTENT_CHARS: usize = 240;

/// Tag prefix carrying the content fingerprint used for exact dedupe.
const CONTENT_HASH_TAG_PREFIX: &str = "contenthash:";

/// The longest content accepted for `memory_type`.
pub fn content_limit(memory_type: MemoryType) -> usize {
    match memory_type {
        MemoryType::Rule => MAX_RULE_CONTENT_CHARS,
        _ => MAX_EXTRACTED_CONTENT_CHARS,
    }
}

/// Stable 64-bit FNV-1a over normalised content.
///
/// Deliberately not `DefaultHasher`: that is seeded and explicitly not stable
/// across builds, so fingerprints written by one binary would not match those
/// written by the next and dedupe would silently stop working after an upgrade.
/// FNV-1a is fixed by its constants, so a hash written today still matches
/// tomorrow.
fn content_fingerprint(content: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut last_was_space = false;
    for ch in content.trim().chars() {
        // Normalise so that whitespace and case differences -- which the LLM
        // reintroduces freely when re-describing the same fact -- do not defeat
        // the fingerprint.
        let ch = if ch.is_whitespace() {
            if last_was_space {
                continue;
            }
            last_was_space = true;
            ' '
        } else {
            last_was_space = false;
            ch.to_ascii_lowercase()
        };
        let mut buffer = [0u8; 4];
        for byte in ch.encode_utf8(&mut buffer).as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// The dedupe tag for `content`.
pub fn content_hash_tag(content: &str) -> String {
    format!(
        "{CONTENT_HASH_TAG_PREFIX}{:016x}",
        content_fingerprint(content)
    )
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
///
/// The instructions carry more weight than they look like they should, because
/// a `rule` written here is injected into every later system prompt without
/// review. Two failures came directly from the earlier version saying only
/// "a concise statement of the fact/decision/preference":
///
/// 1. WHOSE BEHAVIOUR. It never said a rule constrains the ASSISTANT. Given a
///    turn where the user corrects the assistant, the model wrote the user's
///    instruction back out as a prohibition -- `Avoid: <the thing the user
///    asked for>` -- inverting the intent of the correction. The caps do not
///    help here: an inverted rule is short and passes them cleanly.
/// 2. WHAT A MEMORY IS. Nothing said the content had to be a statement rather
///    than an excerpt, so pasted documents came back verbatim as memories.
///    [`MAX_EXTRACTED_CONTENT_CHARS`] now rejects those, but a rejected
///    extraction is still a wasted call and a lost memory -- better not to ask
///    for one.
///
/// `already_recorded` lists corrections the fast keyword detector already
/// captured for these turns. They are shown to the model so it reports only the
/// ones that pass missed: both detectors write to the same place, so a
/// re-reported correction is a duplicate rather than a second opinion.
pub fn build_extraction_prompt(messages: &[String], already_recorded: &[String]) -> String {
    let conversation = messages.join("\n---\n");
    let already = if already_recorded.is_empty() {
        String::new()
    } else {
        let list = already_recorded
            .iter()
            .map(|c| format!("- {}", c.chars().take(200).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "\nThese corrections from this conversation have ALREADY been recorded. Do not report \
             them again, even in different words. Report a \"correction\" only if the user \
             corrected the assistant in a way NOT covered below:\n{list}\n"
        )
    };
    format!(
        r#"Analyse the following conversation and extract any important memories.

You are writing notes for an AI assistant to read in future conversations.
Write each memory as a short statement in your own words. Never copy a
document, file, code block, or long passage out of the conversation: if
something cannot be summarised in a sentence or two, it does not belong here.

For each memory, return a JSON object with:
- "content": one or two sentences, at most {MAX_EXTRACTED_CONTENT_CHARS} characters
- "memory_type": one of "fact", "decision", "correction", "pattern", "preference", "rule"
- "tags": a list of short keyword tags

"rule" and "correction" describe how the ASSISTANT should behave, never what
the user should do. Write them as an instruction addressed to the assistant,
and keep them under {MAX_RULE_CONTENT_CHARS} characters.

When the user corrects the assistant, the rule is the corrected behaviour the
assistant should adopt -- not a prohibition on what the user asked for. If the
user says "always run the tests before pushing", the rule is "Run the tests
before pushing", never "Avoid running the tests before pushing".

Return a JSON array of these objects. If there is nothing worth remembering, return an empty array `[]`.
{already}
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
        // Dropped, not truncated. A cut-off document is still a document, and
        // storing half of one keeps every downstream cost -- prompt bytes,
        // recall scan, duplicate storms -- while destroying whatever meaning
        // the text had.
        let limit = content_limit(memory_type);
        let length = raw.content.chars().count();
        if length > limit {
            tracing::warn!(
                memory_type = %memory_type,
                length,
                limit,
                "discarding over-long extracted memory; content looks like a pasted document rather than a learned memory"
            );
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
    graph: &dyn MemoryTrait,
    memories: &[ExtractedMemory],
    session_id: &str,
) -> Result<usize, MemoryError> {
    let mut stored = 0usize;

    for mem in memories {
        // Exact fingerprint first: one tag lookup, and it is the check that
        // actually catches a re-pasted document. The containment check below
        // cannot, because it finds candidates by full-text searching the whole
        // content -- fuzzy, unbounded, and weakest on exactly the large inputs
        // that produced the duplicate storms. It is also now bounded by a term
        // cap in `keyword_candidates`, so it can miss outright.
        let hash_tag = content_hash_tag(&mem.content);
        let by_hash = SearchFilter {
            tags: vec![hash_tag.clone()],
            ..Default::default()
        };
        // Re-check the tag on each hit: the tag index is tokenised, so a search
        // returns candidates rather than exact matches.
        if graph
            .search_memories(&by_hash)?
            .iter()
            .any(|existing| existing.tags.contains(&hash_tag))
        {
            continue;
        }

        // Retained for near-duplicates the fingerprint cannot see: a restatement
        // that adds a clause is not byte-identical but is still redundant.
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
        tags.push(hash_tag);

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
#[path = "extraction_tests.rs"]
mod tests;
