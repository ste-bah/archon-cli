//! Issue #171 Part 6 — session-scoped cache for agent memory recall.
//!
//! `with_recalled_memory` ran every one of an agent definition's
//! `recall_queries` against the memory store on every spawn, so a fan-out of
//! the same agent type repeated identical queries N times for an identical
//! answer. The cache holds the **rendered** `<agent-memory>` block, not the raw
//! rows, so a hit costs one `Arc` clone.
//!
//! ## Staleness policy
//!
//! Two mechanisms, deliberately layered:
//!
//! 1. **Exact invalidation on the in-repo writer.** Recall matches on
//!    `agent:{type}` + `scope:{scope}` with `require_all_tags`, and
//!    [`super::save_agent_memory`] is the only production writer that emits
//!    those tags. The subagent executor calls it from a single site
//!    (PRESERVE-D8) and invalidates the matching key immediately afterwards, so
//!    a memory write followed by a spawn never serves the pre-write block.
//! 2. **A short TTL as a backstop.** Nothing stops an out-of-band writer (the
//!    memory MCP server, a future extractor) from storing a row with those tags
//!    without going through the executor. The TTL bounds how long such a write
//!    can go unseen to [`DEFAULT_TTL`]; it is not the primary correctness
//!    mechanism.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use archon_memory::MemoryTrait;

use super::store::load_agent_memory;
use super::tags::scope_tag;
use crate::agents::definition::AgentMemoryScope;

/// Backstop lifetime for a cached recall block.
///
/// Sized against what it is protecting: a fan-out burst spawns its agents
/// within milliseconds of each other, so seconds are ample to collapse the
/// duplicate queries, while an out-of-band memory write is visible to the next
/// spawn a few seconds later at worst. Writes made through the executor are not
/// subject to this window at all — they invalidate exactly.
pub const DEFAULT_TTL: Duration = Duration::from_secs(30);

/// `(agent_type, scope_tag, recall_queries)` — the complete set of inputs
/// `load_agent_memory` reads. `recall_queries` is part of the key because a
/// registry reload can change an agent type's queries without changing its name.
type RecallKey = (String, String, String);

struct CachedBlock {
    stored_at: Instant,
    /// `None` records "this agent type has no matching memories", which is as
    /// worth caching as a hit — that is the common case for a fresh session.
    block: Option<Arc<str>>,
}

/// Observed cache behaviour, for spawn fixtures and bench reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecallCacheStats {
    /// Lookups served from cache (no memory-store queries).
    pub hits: usize,
    /// Lookups that queried the memory store (cold, invalidated, or expired).
    pub misses: usize,
    /// Total `recall_queries` executed against the memory store.
    pub queries_run: usize,
}

/// Session-scoped cache of rendered `<agent-memory>` blocks.
#[derive(Debug)]
pub struct AgentMemoryRecallCache {
    ttl: Duration,
    entries: Mutex<HashMap<RecallKey, CachedBlock>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
    queries_run: AtomicUsize,
}

impl std::fmt::Debug for CachedBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CachedBlock")
            .field("age_ms", &self.stored_at.elapsed().as_millis())
            .field("has_block", &self.block.is_some())
            .finish()
    }
}

impl Default for AgentMemoryRecallCache {
    fn default() -> Self {
        Self::with_ttl(DEFAULT_TTL)
    }
}

impl AgentMemoryRecallCache {
    /// Construct a cache with the [`DEFAULT_TTL`] backstop.
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct a cache with an explicit backstop TTL (tests, tuning).
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            entries: Mutex::new(HashMap::new()),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
            queries_run: AtomicUsize::new(0),
        }
    }

    /// Return the rendered `<agent-memory>` block for this agent type, running
    /// the recall queries only on a miss.
    ///
    /// `None` means there is nothing to append — either no scope, no queries, or
    /// no matching memories.
    pub fn block(
        &self,
        agent_type: &str,
        recall_queries: &[String],
        memory: &dyn MemoryTrait,
        memory_scope: Option<&AgentMemoryScope>,
    ) -> Option<Arc<str>> {
        if recall_queries.is_empty() || memory_scope.is_none() {
            return None;
        }
        let key = recall_key(agent_type, recall_queries, memory_scope);

        if let Ok(entries) = self.entries.lock()
            && let Some(entry) = entries.get(&key)
            && entry.stored_at.elapsed() < self.ttl
        {
            self.hits.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(
                agent = agent_type,
                queries = recall_queries.len(),
                "agent-memory recall cache hit"
            );
            return entry.block.clone();
        }

        self.misses.fetch_add(1, Ordering::Relaxed);
        self.queries_run
            .fetch_add(recall_queries.len(), Ordering::Relaxed);
        let memories = load_agent_memory(agent_type, recall_queries, memory, memory_scope);
        let block: Option<Arc<str>> = if memories.is_empty() {
            None
        } else {
            Some(Arc::from(format!(
                "<agent-memory>\n{}\n</agent-memory>",
                memories.join("\n---\n")
            )))
        };

        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(
                key,
                CachedBlock {
                    stored_at: Instant::now(),
                    block: block.clone(),
                },
            );
        }
        block
    }

    /// Drop every cached block for `(agent_type, memory_scope)`.
    ///
    /// Called immediately after a `save_agent_memory` write so the next spawn
    /// re-runs recall. Recall-query variants of the same agent type all key on
    /// the same `(type, scope)` prefix, so they are dropped together.
    pub fn invalidate(&self, agent_type: &str, memory_scope: Option<&AgentMemoryScope>) {
        let Some(scope) = memory_scope else {
            return;
        };
        let scope = scope_tag(scope);
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|(cached_type, cached_scope, _), _| {
                cached_type != agent_type || cached_scope != &scope
            });
        }
    }

    /// Snapshot the hit/miss/query counters.
    pub fn stats(&self) -> RecallCacheStats {
        RecallCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            queries_run: self.queries_run.load(Ordering::Relaxed),
        }
    }
}

fn recall_key(
    agent_type: &str,
    recall_queries: &[String],
    memory_scope: Option<&AgentMemoryScope>,
) -> RecallKey {
    (
        agent_type.to_string(),
        memory_scope.map(scope_tag).unwrap_or_default(),
        recall_queries.join("\u{1f}"),
    )
}
