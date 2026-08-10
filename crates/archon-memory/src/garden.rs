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
mod apply;
mod budget;
mod config;
mod consolidation;
mod phases;
mod provenance;
mod reporting;
mod retirement;
mod rule_retirement;
mod run_lock;
mod scheduling;

pub use adjudication::{Adjudication, ReviewPair, apply_adjudicated_merges};
pub use apply::{
    ChangeOutcome, apply_memory_retirement, apply_rule_retirement, apply_semantic_consolidation,
    derived_memory_id, rollback_memory_retirement, rollback_rule_retirement,
    rollback_semantic_consolidation,
};
pub use budget::{BudgetLedger, GardenBudget};
pub use config::GardenConfig;
// The threshold default is re-exported to the garden's own tests, which pin the
// shipped value against the adjudicator's per-run cap. Reachable from `super::`
// there, as it was before the config moved to a child module.
#[cfg(test)]
use config::default_auto_adjudicate_min_pairs;
pub use consolidation::{
    ConsolidationSource, SemanticConsolidationCandidate, derived_memory_tags,
    semantic_consolidation_candidates,
};
pub use provenance::{
    DERIVED_TAG, Ineligible, MemoryProvenance, compatible_clusters, ineligible_reason,
    provenance_compatible,
};
pub use reporting::{format_garden_stats, generate_briefing};
pub use retirement::{PrunePolicy, RetirementCandidate, RetirementReason};
pub use rule_retirement::{
    RuleObservation, RuleOrigin, RuleRetirementCandidate, RuleRetirementEvidence,
    RuleRetirementPolicy, rule_retirement_candidates,
};
pub use run_lock::{
    GARDEN_RUN_LOCK_FILE, RunLockOutcome, log_declined, log_unavailable, run_lock_path,
    with_run_lock,
};
pub use scheduling::{
    GardenRunPolicy, ScheduledRun, run_scheduled_consolidation, should_run_scheduled,
};

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
    /// The pass stopped at its work or time ceiling with candidates left.
    ///
    /// Distinct from having finished, and the distinction matters: a store that
    /// exhausts its budget on every pass never reaches a fixed point, and the
    /// counts alone cannot say so -- "10 merged" reads identically whether ten
    /// was all there was or the first ten of four hundred.
    ///
    /// Always `false` for the interactive paths, which run unbounded.
    #[serde(default)]
    pub budget_exhausted: bool,
    /// Memories this pass declined to delete, offered for review.
    ///
    /// Empty for a pass running under [`PrunePolicy::Delete`], which prunes
    /// directly and reports the count in `stale_pruned` / `overflow_pruned`.
    /// Populated instead of those counts for a scheduled pass, which never
    /// deletes. The memories named here are still live and untouched.
    #[serde(default)]
    pub retirement_candidates: Vec<RetirementCandidate>,
    /// Semantic memories this pass proposed writing, with their sources.
    ///
    /// Nothing has been written. Each is a cluster of provenance-compatible
    /// memories that restate one another; applying the proposal is a separate,
    /// governed act.
    #[serde(default)]
    pub consolidation_candidates: Vec<SemanticConsolidationCandidate>,
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
///
/// Unbounded, and permitted to delete: this is the path a person started and is
/// watching, and it behaves exactly as it always has.
pub fn consolidate_with_run_id(
    graph: &dyn MemoryTrait,
    config: &GardenConfig,
    run_id: &str,
) -> Result<GardenReport, MemoryError> {
    consolidate_with_policy(graph, config, run_id, GardenRunPolicy::interactive())
}

/// Run one consolidation pass under an explicit work ceiling and prune policy.
///
/// The policy is a parameter rather than a mode flag because the two callers
/// want genuinely different behaviour, not the same behaviour with a switch:
/// `/garden` deletes what it decides to delete, and a scheduled pass may not
/// delete at all. Making that explicit at the call site means a reader of either
/// one can see which they are looking at.
pub fn consolidate_with_policy(
    graph: &dyn MemoryTrait,
    config: &GardenConfig,
    run_id: &str,
    policy: GardenRunPolicy,
) -> Result<GardenReport, MemoryError> {
    if run_id.is_empty() {
        return Err(MemoryError::Database(
            "garden run_id must not be empty".to_string(),
        ));
    }
    let start = Instant::now();
    let mut ledger = BudgetLedger::new(policy.budget);
    let mut retirement_candidates: Vec<RetirementCandidate> = Vec::new();
    let total_before = graph.memory_count()?;

    // Read BEFORE `phase_record_timestamp` overwrites it at the end of this run.
    let previous_run = read_last_run(graph)?;

    let importance_decayed = phase_importance_decay(
        graph,
        config.importance_decay_per_day,
        run_id,
        previous_run,
        &mut ledger,
    )?;
    info!(importance_decayed, "phase 1: importance decay complete");

    let stale = phase_staleness_prune(
        graph,
        config.staleness_days,
        config.staleness_importance_floor,
        policy.prune,
        &mut ledger,
    )?;
    let stale_pruned = stale.pruned;
    retirement_candidates.extend(stale.candidates);
    info!(
        stale_pruned,
        proposed = retirement_candidates.len(),
        "phase 2: staleness prune complete"
    );

    let lexical_merged = phase_dedup(graph, config.dedup_similarity_threshold, &mut ledger)?;
    // Semantic pass second, and with the remaining budget: the lexical pass is
    // free and exact, so let it take the easy cases before spending vector
    // lookups. A store with no vector search reports `None` here, and the
    // lexical result still stands on its own.
    let (semantic_merged, review_pairs) = phase_semantic_dedup(
        graph,
        config.semantic_dedup_max_distance,
        config.semantic_review_max_distance,
        DEDUP_MERGE_BUDGET.saturating_sub(lexical_merged),
        &mut ledger,
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

    let fragments_merged = phase_fragment_merge(graph, &mut ledger)?;
    info!(fragments_merged, "phase 4: fragment merge complete");

    let overflow = phase_overflow_prune(graph, config.max_memories, policy.prune, &mut ledger)?;
    let overflow_pruned = overflow.pruned;
    retirement_candidates.extend(overflow.candidates);
    info!(overflow_pruned, "phase 5: overflow prune complete");

    // Generative half, and the only phase that proposes rather than changes.
    // Runs after the merges so it clusters what deduplication has already
    // settled, rather than proposing a consolidation of rows that are about to
    // be folded together anyway.
    //
    // Skipped entirely for an interactive pass: `/garden` reports what it DID,
    // and a list of proposals nobody can act on from that surface would read as
    // work performed. Proposals belong to the governed path.
    let consolidation_candidates = if policy.prune.may_delete() {
        Vec::new()
    } else {
        // A pass must not propose forgetting a memory and enshrining it in the
        // same breath, so anything already offered for retirement is out of the
        // clustering pool.
        let already_proposed: std::collections::HashSet<String> = retirement_candidates
            .iter()
            .map(|candidate| candidate.memory_id.clone())
            .collect();
        consolidation::phase_semantic_consolidation(
            graph,
            config.consolidation_min_cluster_size,
            chrono::Duration::days(config.consolidation_max_span_days),
            config.consolidation_min_word_overlap,
            &already_proposed,
            &mut ledger,
        )?
    };
    info!(
        proposed = consolidation_candidates.len(),
        "phase 6: semantic consolidation proposals complete"
    );

    // Recorded even when the budget stopped this pass short. The timestamp's
    // only reader is the decay bill, and skipping it would make the next pass
    // charge this pass's span a second time -- see `phase_record_timestamp`.
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
        budget_exhausted: ledger.exhausted(),
        retirement_candidates,
        consolidation_candidates,
    };
    if report.budget_exhausted {
        // WARN, not info. One exhausted pass is unremarkable; every pass
        // exhausting means the store never reaches a fixed point, and the counts
        // in the report cannot distinguish the two.
        warn!(
            reversible_ops = ledger.spent_reversible(),
            deletions = ledger.spent_deletions(),
            proposals = ledger.spent_proposals(),
            duration_ms,
            "garden: consolidation stopped at its work budget with candidates remaining"
        );
    }
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

/// Whether this run's review band has earned an automatic adjudication call.
///
/// `pending_pairs` is [`GardenReport::review_pairs`] length from the pass that
/// just ran, which is the whole accumulated band rather than a delta: the
/// semantic phase re-derives it from the store every time, and the band writes
/// nothing, so anything left unjudged is simply reported again.
///
/// A predicate rather than a call site condition because it is the whole policy
/// this setting expresses, and a policy about spending money and mutating memory
/// should be testable without a provider.
pub fn should_auto_adjudicate(config: &GardenConfig, pending_pairs: usize) -> bool {
    // An empty band is checked explicitly rather than left to the comparison:
    // `auto_adjudicate_min_pairs = 0` is a legal way to say "always", and
    // without this it would also mean "call the model about nothing".
    config.auto_adjudicate_review_band
        && pending_pairs > 0
        && pending_pairs >= config.auto_adjudicate_min_pairs
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

#[cfg(test)]
#[path = "garden/auto_adjudicate_tests.rs"]
mod auto_adjudicate_tests;
