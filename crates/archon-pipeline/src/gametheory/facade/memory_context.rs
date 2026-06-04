use std::sync::Arc;

use crate::leann_searcher::LeannSearcher;
use archon_memory::{MemoryTrait, SearchFilter};

use super::super::registry::GAMETHEORY_AGENTS;
use super::TAG_GAMETHEORY_PIPELINE;

/// Optional memory backends used for gametheory prompt enrichment.
#[derive(Clone, Default)]
pub struct GameTheoryMemoryContext {
    pub memory: Option<Arc<dyn MemoryTrait>>,
    pub leann_searcher: Option<Arc<dyn LeannSearcher>>,
    pub debug: bool,
}

impl GameTheoryMemoryContext {
    pub fn new(
        memory: Arc<dyn MemoryTrait>,
        leann_searcher: Option<Arc<dyn LeannSearcher>>,
        debug: bool,
    ) -> Self {
        Self {
            memory: Some(memory),
            leann_searcher,
            debug,
        }
    }
}
/// Source-of-truth audit for memory recall performed before an agent call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecallAudit {
    pub agent_key: String,
    pub memory_keys: Vec<String>,
    pub cozo_hits: usize,
    pub leann_hits: usize,
}

#[derive(Debug, Clone)]
pub(super) struct RecalledContext {
    pub(super) text: String,
    pub(super) audit: MemoryRecallAudit,
}

fn agent_memory_keys(agent_key: &str) -> &'static [&'static str] {
    GAMETHEORY_AGENTS
        .iter()
        .find(|agent| agent.key == agent_key)
        .map(|agent| agent.memory_keys)
        .unwrap_or(&[])
}

pub(super) fn recall_prior_context_for_agent(
    agent_key: &str,
    memory_ctx: &GameTheoryMemoryContext,
) -> RecalledContext {
    let memory_keys = agent_memory_keys(agent_key);
    let mut cozo_hits = 0usize;
    let mut leann_hits = 0usize;
    let mut parts = Vec::new();

    for &memory_key in memory_keys {
        let mut key_parts = Vec::new();

        if let Some(memory) = memory_ctx.memory.as_ref() {
            let filter = SearchFilter {
                tags: vec![TAG_GAMETHEORY_PIPELINE.to_string()],
                ..Default::default()
            };
            if let Ok(memories) = memory.search_memories(&filter) {
                for m in memories {
                    if m.title == memory_key {
                        cozo_hits += 1;
                        key_parts.push(m.content);
                    }
                }
            }
        }

        if key_parts.is_empty()
            && let Some(leann) = memory_ctx.leann_searcher.as_ref()
        {
            let fallback = leann.search(memory_key);
            if !fallback.trim().is_empty() {
                leann_hits += 1;
                key_parts.push(fallback);
            }
        }

        if !key_parts.is_empty() {
            parts.push(format!("#### {memory_key}\n\n{}", key_parts.join("\n\n")));
        }
    }

    RecalledContext {
        text: parts.join("\n\n---\n\n"),
        audit: MemoryRecallAudit {
            agent_key: agent_key.to_string(),
            memory_keys: memory_keys.iter().map(|key| key.to_string()).collect(),
            cozo_hits,
            leann_hits,
        },
    }
}
