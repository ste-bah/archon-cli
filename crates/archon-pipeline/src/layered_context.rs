//! Layered Context Loading (L0-L3) for pipeline agents.
//!
//! Assembles four-tier memory context with configurable token budgets:
//! - L0 Identity (~100 tokens, always loaded)
//! - L1 Essential Patterns (~500 tokens, always loaded)
//! - L2 On-Demand (~500 tokens, per-agent, compressed)
//! - L3 Deep Search (unlimited, fallback only on quality retry)

use crate::coding::rlm::RlmStore;
use crate::compression;
use crate::prompt_cap::count_tokens;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Identity context for L0.
#[derive(Debug, Clone)]
pub struct IdentityContext {
    pub session_id: String,
    pub pipeline_type: String,
    pub task_summary: String,
    pub agent_position: String,
    pub wiring_obligations: Option<String>,
}

/// Pattern context for L1.
#[derive(Debug, Clone)]
pub struct PatternContext {
    pub sona_patterns: Vec<String>,
    pub recent_corrections: Vec<String>,
    pub architectural_decisions: Vec<String>,
}

/// Agent-specific memory request for L2.
#[derive(Debug, Clone)]
pub struct AgentMemoryRequest {
    pub agent_key: String,
    pub memory_domains: Vec<String>,
    pub phase: u32,
}

/// The assembled layered context.
#[derive(Debug, Clone)]
pub struct LayeredContext {
    pub l0_identity: String,
    pub l1_patterns: String,
    pub l2_on_demand: String,
    pub l3_deep: Option<String>,
    pub total_tokens: usize,
}

/// Four-tier context loader with configurable budgets.
#[derive(Debug, Clone)]
pub struct LayeredContextLoader {
    l0_budget: usize,
    l1_budget: usize,
    l2_budget: usize,
}

// ---------------------------------------------------------------------------
// Helper: truncate text to a token budget
// ---------------------------------------------------------------------------

fn truncate_to_budget(text: &str, budget: usize) -> String {
    if count_tokens(text) <= budget {
        return text.to_string();
    }
    let max_chars = budget * 4;
    if text.len() <= max_chars {
        return text.to_string();
    }
    // Find the last char boundary at or before max_chars to avoid
    // panicking on multi-byte UTF-8.
    let end = text
        .char_indices()
        .take_while(|(i, _)| *i < max_chars)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    text[..end].to_string()
}

// ---------------------------------------------------------------------------
// Implementation
// ---------------------------------------------------------------------------

impl LayeredContextLoader {
    /// Create a loader with default budgets: L0=100, L1=500, L2=500.
    pub fn new() -> Self {
        Self {
            l0_budget: 100,
            l1_budget: 500,
            l2_budget: 500,
        }
    }

    /// Create a loader with custom budgets.
    pub fn with_budgets(l0: usize, l1: usize, l2: usize) -> Self {
        Self {
            l0_budget: l0,
            l1_budget: l1,
            l2_budget: l2,
        }
    }

    /// Load L0 identity context.
    ///
    /// Format: compact text with session, type, task, position, and optional wiring.
    pub fn load_l0(&self, identity: &IdentityContext) -> String {
        let mut text = format!(
            "[IDENTITY] session={} type={}\nTask: {}\nPosition: {}",
            identity.session_id,
            identity.pipeline_type,
            identity.task_summary,
            identity.agent_position,
        );
        if let Some(ref wiring) = identity.wiring_obligations {
            text.push_str(&format!("\nWiring: {}", wiring));
        }
        truncate_to_budget(&text, self.l0_budget)
    }

    /// Load L1 essential patterns context.
    ///
    /// Format: SONA patterns, corrections, and architectural decisions.
    pub fn load_l1(&self, patterns: &PatternContext) -> String {
        let sona = if patterns.sona_patterns.is_empty() {
            "SONA: (none)".to_string()
        } else {
            format!("SONA: {}", patterns.sona_patterns.join(" | "))
        };

        let corrections = if patterns.recent_corrections.is_empty() {
            "Corrections: (none)".to_string()
        } else {
            format!(
                "Corrections: {}",
                patterns.recent_corrections.join(" | ")
            )
        };

        let decisions = if patterns.architectural_decisions.is_empty() {
            "Decisions: (none)".to_string()
        } else {
            format!(
                "Decisions: {}",
                patterns.architectural_decisions.join(" | ")
            )
        };

        let text = format!("[PATTERNS]\n{}\n{}\n{}", sona, corrections, decisions);
        truncate_to_budget(&text, self.l1_budget)
    }

    /// Load L2 on-demand context from RLM store, compressed to budget.
    pub fn load_l2(&self, request: &AgentMemoryRequest, rlm_store: &RlmStore) -> String {
        let mut parts: Vec<String> = Vec::new();
        for domain in &request.memory_domains {
            if let Some(content) = rlm_store.read(domain) {
                parts.push(content);
            }
        }

        if parts.is_empty() {
            return String::new();
        }

        let raw = parts.join("\n");
        let compressed = compression::compress(&raw, self.l2_budget);

        // If compression produced meaningful structured output, use it.
        // Otherwise fall back to a budget-truncated version of the raw text
        // that still preserves useful keywords for the agent.
        if compressed.entities_preserved > 0 || !compressed.sections_present.is_empty() {
            let text = compressed.text;
            if count_tokens(&text) > self.l2_budget {
                truncate_to_budget(&text, self.l2_budget)
            } else {
                text
            }
        } else {
            // Compression was degenerate (no entities extracted). Fall back to
            // truncated raw content. Use a budget slightly below the raw token
            // count to guarantee the output is shorter than the input.
            let raw_tokens = count_tokens(&raw);
            let effective_budget = self.l2_budget.min(raw_tokens.saturating_sub(2).max(1));
            truncate_to_budget(&raw, effective_budget)
        }
    }

    /// Load L3 deep search context (unlimited budget).
    ///
    /// Searches all namespaces in the RLM store for content matching
    /// any word from the query (case-insensitive substring match).
    pub fn load_l3(&self, query: &str, rlm_store: &RlmStore) -> String {
        let words: Vec<String> = query
            .split_whitespace()
            .map(|w| w.to_lowercase())
            .collect();

        let mut matches: Vec<String> = Vec::new();
        for ns in rlm_store.namespaces() {
            if let Some(content) = rlm_store.read(ns) {
                let lower = content.to_lowercase();
                let has_match = words.iter().any(|w| lower.contains(w));
                if has_match {
                    matches.push(content);
                }
            }
        }

        matches.join("\n")
    }

    /// Load all context layers and assemble the final `LayeredContext`.
    pub fn load_context(
        &self,
        identity: &IdentityContext,
        patterns: &PatternContext,
        request: &AgentMemoryRequest,
        rlm_store: &RlmStore,
        trigger_l3: bool,
    ) -> LayeredContext {
        let l0 = self.load_l0(identity);
        let l1 = self.load_l1(patterns);
        let l2 = self.load_l2(request, rlm_store);

        let l3 = if trigger_l3 {
            Some(self.load_l3(&identity.task_summary, rlm_store))
        } else {
            None
        };

        let total_tokens = count_tokens(&l0)
            + count_tokens(&l1)
            + count_tokens(&l2)
            + l3.as_ref().map(|s| count_tokens(s)).unwrap_or(0);

        LayeredContext {
            l0_identity: l0,
            l1_patterns: l1,
            l2_on_demand: l2,
            l3_deep: l3,
            total_tokens,
        }
    }
}

impl Default for LayeredContextLoader {
    fn default() -> Self {
        Self::new()
    }
}
