//! Memory Garden — autonomous memory consolidation engine.
//!
//! Deduplicates, prunes stale memories, decays importance, merges fragments,
//! and generates session briefings.

use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::access::MemoryTrait;
use crate::types::{Memory, MemoryError, MemoryType, SearchFilter};

mod adjudication;
mod phases;
mod reporting;

pub use adjudication::{Adjudication, ReviewPair, apply_adjudicated_merges};
pub use reporting::{format_garden_stats, generate_briefing};

use phases::{
    DEDUP_MERGE_BUDGET, phase_dedup, phase_fragment_merge, phase_importance_decay,
    phase_overflow_prune, phase_record_timestamp, phase_semantic_dedup, phase_staleness_prune,
    read_last_run,
};

/// Memory types that are safe to prune, decay, merge, and deduplicate.
const PRUNEABLE_TYPES: [MemoryType; 5] = [
    MemoryType::Fact,
    MemoryType::Decision,
    MemoryType::Correction,
    MemoryType::Pattern,
    MemoryType::Preference,
];

/// Cosine distance below which two memories are merged automatically.
///
/// MEASURED, not chosen. `tests/semantic_distance_calibration.rs` embeds real
/// restatements from a real store and reports the distributions:
///
/// * restatements of one instruction: 0.09 - 0.35
/// * genuinely distinct claims:       0.32 and up
///
/// Those ranges OVERLAP, so no single threshold separates them -- an earlier
/// attempt at 0.08 merged nothing on a real store, and anything loose enough to
/// catch the restatements would also merge "deploy to eu-west-2" with "never
/// deploy to us-east-1". 0.15 sits clear of the 0.32 floor with margin, and
/// takes the unambiguous cases only.
fn default_semantic_dedup_max_distance() -> f64 {
    0.15
}

/// Upper bound of the review band.
///
/// Between the merge distance and this, two memories are probably about the
/// same thing but not provably the same claim. They are REPORTED as
/// [`ReviewPair`]s and otherwise untouched, so the pairing is visible without a
/// judgement being made for you. This is the band an adjudicator decides, and
/// the band where a naive threshold does its damage.
fn default_semantic_review_max_distance() -> f64 {
    0.35
}

/// Configuration for the memory garden consolidation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GardenConfig {
    pub auto_consolidate: bool,
    pub min_hours_between_runs: u32,
    pub dedup_similarity_threshold: f32,
    /// Maximum cosine DISTANCE at which two memories are treated as saying the
    /// same thing. Smaller is more similar; 0.0 is identical.
    ///
    /// Separate from `dedup_similarity_threshold`, which is Jaccard word
    /// overlap and only ever catches near-verbatim copies. This one catches
    /// restatements, which is where the real duplication comes from: one
    /// instruction can accumulate a dozen differently-worded memories across
    /// turns and writers.
    ///
    /// Tight by default, because merging deletes. Two memories about the same
    /// subject are not necessarily the same claim.
    #[serde(default = "default_semantic_dedup_max_distance")]
    pub semantic_dedup_max_distance: f64,
    /// Upper bound of the review band; pairs between the merge distance and
    /// this are reported for adjudication, never merged automatically. Set
    /// equal to the merge distance to stop reporting them.
    #[serde(default = "default_semantic_review_max_distance")]
    pub semantic_review_max_distance: f64,
    pub staleness_days: u32,
    pub staleness_importance_floor: f64,
    pub importance_decay_per_day: f64,
    pub max_memories: usize,
    pub briefing_limit: usize,
}

impl Default for GardenConfig {
    fn default() -> Self {
        Self {
            auto_consolidate: true,
            min_hours_between_runs: 24,
            dedup_similarity_threshold: 0.92,
            semantic_dedup_max_distance: default_semantic_dedup_max_distance(),
            semantic_review_max_distance: default_semantic_review_max_distance(),
            staleness_days: 30,
            staleness_importance_floor: 0.3,
            importance_decay_per_day: 0.01,
            max_memories: 5000,
            briefing_limit: 15,
        }
    }
}

/// Results of a consolidation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GardenReport {
    pub duplicates_merged: usize,
    pub stale_pruned: usize,
    pub importance_decayed: usize,
    pub fragments_merged: usize,
    pub overflow_pruned: usize,
    pub total_memories_before: usize,
    pub total_memories_after: usize,
    pub duration_ms: u64,
    /// Pairs that landed in the review band: probably the same subject, not
    /// provably the same claim. Nothing has been done to them. A caller with a
    /// provider can judge them and apply the verdicts with
    /// [`apply_adjudicated_merges`].
    #[serde(default)]
    pub review_pairs: Vec<ReviewPair>,
    /// The semantic pass did not run, because this store has no vector search.
    ///
    /// Distinct from having run and merged nothing. Without it, the only
    /// evidence of a skipped pass was `duplicates_merged` holding the lexical
    /// count alone, which reads exactly like a clean store -- and every Archon
    /// process but the first reads memory over TCP, so the skip was the norm.
    ///
    /// `#[serde(default)]` because reports persisted before this field existed
    /// must still deserialize; `false` reads them as "the pass ran", which is
    /// the same claim they were already making.
    #[serde(default)]
    pub semantic_pass_unavailable: bool,
}

// ── public API ───────────────────────────────────────────────

/// Run a full consolidation pass across all six phases.
pub fn consolidate(
    graph: &dyn MemoryTrait,
    config: &GardenConfig,
) -> Result<GardenReport, MemoryError> {
    consolidate_with_run_id(graph, config, &uuid::Uuid::new_v4().to_string())
}

/// Run or retry one logical consolidation pass using stable mutation provenance.
pub fn consolidate_with_run_id(
    graph: &dyn MemoryTrait,
    config: &GardenConfig,
    run_id: &str,
) -> Result<GardenReport, MemoryError> {
    if run_id.is_empty() {
        return Err(MemoryError::Database(
            "garden run_id must not be empty".to_string(),
        ));
    }
    let start = Instant::now();
    let total_before = graph.memory_count()?;

    // Read BEFORE `phase_record_timestamp` overwrites it at the end of this run.
    let previous_run = read_last_run(graph)?;

    let importance_decayed =
        phase_importance_decay(graph, config.importance_decay_per_day, run_id, previous_run)?;
    info!(importance_decayed, "phase 1: importance decay complete");

    let stale_pruned = phase_staleness_prune(
        graph,
        config.staleness_days,
        config.staleness_importance_floor,
    )?;
    info!(stale_pruned, "phase 2: staleness prune complete");

    let lexical_merged = phase_dedup(graph, config.dedup_similarity_threshold)?;
    // Semantic pass second, and with the remaining budget: the lexical pass is
    // free and exact, so let it take the easy cases before spending vector
    // lookups. A store with no vector search reports `None` here, and the
    // lexical result still stands on its own.
    let (semantic_merged, review_pairs) = phase_semantic_dedup(
        graph,
        config.semantic_dedup_max_distance,
        config.semantic_review_max_distance,
        DEDUP_MERGE_BUDGET.saturating_sub(lexical_merged),
    )?;
    let semantic_pass_unavailable = semantic_merged.is_none();
    let duplicates_merged = lexical_merged + semantic_merged.unwrap_or(0);
    // Logged as a separate field rather than folded into `semantic_merged`,
    // because a zero there and an absent pass are the two readings this whole
    // change exists to keep apart.
    info!(
        lexical_merged,
        semantic_merged = semantic_merged.unwrap_or(0),
        semantic_pass_unavailable,
        review_pairs = review_pairs.len(),
        "phase 3: deduplication complete"
    );

    let fragments_merged = phase_fragment_merge(graph)?;
    info!(fragments_merged, "phase 4: fragment merge complete");

    let overflow_pruned = phase_overflow_prune(graph, config.max_memories)?;
    info!(overflow_pruned, "phase 5: overflow prune complete");

    phase_record_timestamp(graph)?;
    info!("phase 6: timestamp recorded");

    let total_after = graph.memory_count()?;
    let duration_ms = start.elapsed().as_millis() as u64;

    let report = GardenReport {
        duplicates_merged,
        stale_pruned,
        importance_decayed,
        fragments_merged,
        overflow_pruned,
        total_memories_before: total_before,
        total_memories_after: total_after,
        duration_ms,
        review_pairs,
        semantic_pass_unavailable,
    };
    info!(?report, semantic_pass_unavailable, "consolidation complete");
    Ok(report)
}

/// Check whether enough time has elapsed since the last consolidation.
pub fn should_auto_consolidate(
    graph: &dyn MemoryTrait,
    min_hours: u32,
) -> Result<bool, MemoryError> {
    let filter = SearchFilter {
        tags: vec!["garden:last_run".into()],
        require_all_tags: true,
        ..SearchFilter::default()
    };
    let results = graph.search_memories(&filter)?;
    let Some(mem) = results.first() else {
        return Ok(true);
    };
    let Ok(last_run) = mem.content.parse::<DateTime<Utc>>() else {
        warn!("could not parse garden:last_run timestamp, re-running");
        return Ok(true);
    };
    let hours_elapsed = (Utc::now() - last_run).num_hours();
    Ok(hours_elapsed >= i64::from(min_hours))
}

fn get_memories_by_type(
    graph: &dyn MemoryTrait,
    memory_type: MemoryType,
) -> Result<Vec<Memory>, MemoryError> {
    let filter = SearchFilter {
        memory_type: Some(memory_type),
        ..SearchFilter::default()
    };
    graph.search_memories(&filter)
}

#[cfg(test)]
#[path = "garden/semantic_dedup_tests.rs"]
mod semantic_dedup_tests;

#[cfg(test)]
#[path = "garden/retry_tests.rs"]
mod retry_tests;
