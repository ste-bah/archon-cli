//! Measuring whether consolidated memories are ever actually read.
//!
//! Applying a consolidation proposal writes a semantic memory. Whether anything
//! then recalls it is the question the R4 promotion gate calls semantic reuse,
//! and it cannot be answered from the write side: a consolidation that is never
//! recalled looks, from the store, exactly like one that is recalled constantly.
//!
//! So this observes the injection path. Every time memories are recalled into a
//! prompt, it records one row per consolidated memory: `cited = true` if that
//! memory reached the prompt, `cited = false` if it did not.
//!
//! # Both halves are written, and that is the whole point
//!
//! Writing only hits would leave the denominator to whatever else happened to
//! emit a retrieval row, and the rate would climb toward 1.0 as consolidations
//! were used once each — the same trap the reversal rate avoids by splitting its
//! numerator and denominator across apply and rollback.
//!
//! Here the denominator is explicit: every consolidated memory that EXISTED when
//! the prompt was built is a row. A consolidation nobody recalls produces a long
//! run of `cited = false` and drags the rate down, which is exactly what it
//! should do.
//!
//! # What `cited` means at this layer
//!
//! It means the memory reached the prompt. Whether the model then used it is not
//! observable here, and claiming otherwise would be the more flattering reading
//! of the same evidence. "Recalled into the prompt" is the strongest honest
//! claim the injection path can support.
//!
//! # These rows stay out of the lesson metrics
//!
//! `lesson_citation_rate` and `reflection_verified_reuse_rate` both select on
//! `rule_injected = true`. These rows carry `rule_injected = false` — they are
//! memories, not injected rules — so they cannot drift into a population that
//! measures something else.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use archon_cognitive::metrics::event::{CognitiveMetricEvent, MetricEventKind};
use archon_memory::MemoryTrait;
use archon_memory::garden::DERIVED_TAG;
use archon_memory::injection::{InjectionObserver, InjectionOutcome, set_injection_observer};
use archon_memory::types::SearchFilter;

use crate::command::garden_metrics::{GardenMetricContext, write_batch};

/// Metric name the rows are written under.
const METRIC: &str = "consolidated_memory_reuse_rate";

/// Most consolidated memories one injection will account for.
///
/// A ceiling on rows per prompt. Above it the observation is SKIPPED rather than
/// truncated: a truncated denominator is a biased sample of which consolidations
/// are being used, and a biased reuse rate is worse than a missing one. The warn
/// says so if it ever fires.
const MAX_CONSOLIDATED_TRACKED: usize = 50;

/// How many observation identities to remember before forgetting all of them.
///
/// The metric store rejects a second write of one event id whose content has
/// moved, and a timestamp always moves. This set is what keeps a repeated
/// injection from attempting that write at all. Cleared wholesale when full
/// rather than evicted one by one: a session that has built this many distinct
/// prompts has moved past the contexts at the bottom of the set.
const MAX_REMEMBERED_OBSERVATIONS: usize = 4096;

pub(crate) struct ConsolidationReuseObserver {
    memory: Arc<dyn MemoryTrait>,
    metrics: GardenMetricContext,
    /// Observation identities already written by this process.
    written: Mutex<HashSet<String>>,
}

impl ConsolidationReuseObserver {
    pub(crate) fn new(memory: Arc<dyn MemoryTrait>, metrics: GardenMetricContext) -> Self {
        Self {
            memory,
            metrics,
            written: Mutex::new(HashSet::new()),
        }
    }

    /// Consolidated memories currently in the store.
    ///
    /// This is the denominator. It is re-read per observation rather than
    /// cached, because a consolidation applied mid-session must start being
    /// measured immediately — and a rollback must stop being measured, or the
    /// rate would keep counting a memory that no longer exists as unused.
    fn consolidated(&self) -> Vec<archon_memory::types::Memory> {
        let filter = SearchFilter {
            tags: vec![DERIVED_TAG.to_string()],
            require_all_tags: true,
            // One more than the ceiling, so exceeding it is detectable rather
            // than silently arriving at exactly the cap.
            limit: Some(MAX_CONSOLIDATED_TRACKED + 1),
            ..SearchFilter::default()
        };
        match self.memory.search_memories(&filter) {
            Ok(rows) => rows,
            Err(error) => {
                tracing::debug!(%error, "consolidated memory reuse: store unreadable");
                Vec::new()
            }
        }
    }

    /// Remember an identity, reporting whether it is new to this process.
    fn claim(&self, id: &str) -> bool {
        let mut written = self
            .written
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if written.len() >= MAX_REMEMBERED_OBSERVATIONS {
            written.clear();
        }
        written.insert(id.to_string())
    }
}

impl InjectionObserver for ConsolidationReuseObserver {
    fn observed(&self, outcome: &InjectionOutcome<'_>) {
        // A cached block is the same prompt situation as the call that filled
        // the cache, and was already recorded then. Skipped here rather than
        // deduped downstream, because the store treats a same-id write whose
        // timestamp has moved as a conflict rather than a replay.
        if outcome.from_cache {
            return;
        }
        let consolidated = self.consolidated();
        if consolidated.is_empty() {
            return;
        }
        if consolidated.len() > MAX_CONSOLIDATED_TRACKED {
            tracing::warn!(
                consolidated = consolidated.len(),
                ceiling = MAX_CONSOLIDATED_TRACKED,
                "consolidated memory reuse: too many consolidated memories to \
                 account for without biasing the denominator; skipping"
            );
            return;
        }

        let injected: HashSet<&str> = outcome
            .injected
            .iter()
            .map(|memory| memory.id.as_str())
            .collect();

        let observations: Vec<(String, bool)> = consolidated
            .iter()
            .map(|memory| (memory.id.clone(), injected.contains(memory.id.as_str())))
            .filter(|(id, _)| self.claim(&observation_id(outcome.context_hash, id)))
            .collect();
        if observations.is_empty() {
            return;
        }

        let context_hash = outcome.context_hash;
        let metrics = &self.metrics;
        write_batch(metrics, |emitter| {
            observations
                .iter()
                .map(|(memory_id, cited)| {
                    build_event(emitter, metrics, context_hash, memory_id, *cited)
                })
                .collect()
        });
    }
}

/// Identity of one observation: this prompt, this consolidated memory.
fn observation_id(context_hash: u64, memory_id: &str) -> String {
    format!("{context_hash:016x}:{memory_id}")
}

fn build_event(
    emitter: &archon_cognitive::metrics::MetricEmitter<'_>,
    context: &GardenMetricContext,
    context_hash: u64,
    memory_id: &str,
    cited: bool,
) -> CognitiveMetricEvent {
    let subject = observation_id(context_hash, memory_id);
    emitter
        .event(
            METRIC,
            MetricEventKind::RetrievalHitObserved,
            &subject,
            chrono::Utc::now(),
        )
        .with_session(context.session_id.as_str(), context.turn_number)
        .with_identity("retrieval_hit_id", subject.as_str())
        .with_identity("lesson_id", memory_id)
        // FALSE on purpose: these are memories, not injected prompt rules, and
        // the two lesson metrics select on `rule_injected = true`. Marking them
        // true would fold consolidation reuse into a rate about something else.
        .with_identity("rule_injected", "false")
        .with_identity("consolidated_memory", "true")
        .with_identity("cited", if cited { "true" } else { "false" })
        .with_identity("prompt_context_id", format!("{context_hash:016x}"))
}

/// Install the observer for this process.
///
/// Called once from session bootstrap. Cheap when no consolidated memories
/// exist: one tag-filtered search per uncached injection, against a path that
/// has just run a full hybrid recall.
pub(crate) fn install(memory: Arc<dyn MemoryTrait>, metrics: GardenMetricContext) {
    set_injection_observer(Arc::new(ConsolidationReuseObserver::new(memory, metrics)));
}

#[cfg(test)]
#[path = "consolidation_reuse_tests.rs"]
mod tests;
