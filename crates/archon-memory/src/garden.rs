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
mod budget;
mod phases;
mod reporting;
mod retirement;
mod run_lock;
mod scheduling;

pub use adjudication::{Adjudication, ReviewPair, apply_adjudicated_merges};
pub use budget::{BudgetLedger, GardenBudget};
pub use reporting::{format_garden_stats, generate_briefing};
pub use retirement::{PrunePolicy, RetirementCandidate, RetirementReason};
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

/// How many pending review-band pairs justify one automatic adjudication call.
///
/// The cost of adjudicating is one LLM round-trip regardless of batch size, so
/// the question is how many judgements that round-trip has to buy. Ten amortises
/// it; paying a session-start round-trip to settle two pairs does not.
///
/// It also sits below the adjudicator's own `MAX_PAIRS_PER_RUN` cap of 20, so a
/// run that fires at the threshold clears the whole accumulated band in a single
/// call instead of leaving a remainder that immediately re-triggers.
fn default_auto_adjudicate_min_pairs() -> usize {
    10
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
    /// Judge the review band automatically once enough pairs have accumulated,
    /// instead of waiting for someone to run `/garden` by hand.
    ///
    /// OFF by default, and the reason is startup latency: automatic
    /// consolidation runs on the session-start path, and adjudication is an LLM
    /// round-trip that would sit between launching Archon and being able to type.
    /// Nothing else on that path calls a model. Left to accumulate the band is
    /// merely untidy; enabled without asking, every opted-out user would pay for
    /// tidiness they did not request, on the one path where waiting is most
    /// obvious.
    ///
    /// `#[serde(default)]` rather than a named default function: an existing
    /// config file that predates this field must read as "off", and `false` is
    /// what `bool`'s `Default` already gives.
    #[serde(default)]
    pub auto_adjudicate_review_band: bool,
    /// Pending review-band pairs required before automatic adjudication fires.
    ///
    /// The threshold is what keeps the setting above from meaning "an LLM call
    /// on every session start": the band has to be worth the round-trip. Ignored
    /// entirely when `auto_adjudicate_review_band` is false.
    #[serde(default = "default_auto_adjudicate_min_pairs")]
    pub auto_adjudicate_min_pairs: usize,
    pub staleness_days: u32,
    pub staleness_importance_floor: f64,
    pub importance_decay_per_day: f64,
    pub max_memories: usize,
    pub briefing_limit: usize,
    /// Run consolidation on a timer, without anyone asking for it.
    ///
    /// OFF by default, and this is the knob that most needs to stay off. Every
    /// other automatic path in the garden is attached to something a person did
    /// -- they launched Archon, or typed `/garden` -- and its results land where
    /// they are looking. A timer detaches consolidation from any of that: it
    /// decays and merges a user's stored memories at an hour they did not
    /// choose, and reports to a log.
    ///
    /// A scheduled pass is deliberately weaker than the manual one. It cannot
    /// delete a memory at all -- pruning becomes a [`RetirementCandidate`] for
    /// review -- and it stops at a fixed work and time ceiling. It is a
    /// maintenance pass, not an autonomous curator.
    ///
    /// `#[serde(default)]` rather than a named function: a config file written
    /// before this field existed must read as "off", and `false` is what `bool`
    /// already gives.
    #[serde(default)]
    pub scheduled_consolidation: bool,
    /// Hours between scheduled passes. Ignored when the above is false.
    #[serde(default = "default_scheduled_interval_hours")]
    pub scheduled_interval_hours: u32,
    /// Most reversible mutations one scheduled pass may make.
    ///
    /// Decays plus merges, counted together, because what is being bounded is
    /// round trips to the store and the store cannot tell them apart. Every
    /// Archon process after the first reaches memory over TCP, so these are
    /// sockets rather than function calls.
    #[serde(default = "default_scheduled_max_reversible_ops")]
    pub scheduled_max_reversible_ops: usize,
    /// Most retirement candidates one scheduled pass may propose for review.
    #[serde(default = "default_scheduled_max_retirement_candidates")]
    pub scheduled_max_retirement_candidates: usize,
    /// Wall-clock ceiling on one scheduled pass, in seconds.
    ///
    /// Nothing is cancelled when it expires; the pass stops taking on new work
    /// at the next unit boundary, which is the only kind of stopping that leaves
    /// the store consistent.
    #[serde(default = "default_scheduled_max_seconds")]
    pub scheduled_max_seconds: u64,
}

/// Once a day, matching `min_hours_between_runs`.
///
/// Consolidation is maintenance, not a reaction to anything. Running it more
/// often spends round trips to reach the same fixed point sooner, and running it
/// less lets the review band and the decay bill accumulate.
fn default_scheduled_interval_hours() -> u32 {
    24
}

/// Enough to finish an ordinary store, small enough to be survivable.
///
/// Sized against the phases' own caps: dedup already stops at
/// `DEDUP_MERGE_BUDGET` (50) and fragment merge at 20, so a pass that merges
/// everything it is allowed to still leaves most of this for decay, which is one
/// write per memory that has aged a day. A store of a few hundred rows finishes
/// inside it; a store far larger stops early and says so, rather than spending
/// an unbounded night on it.
fn default_scheduled_max_reversible_ops() -> usize {
    500
}

/// A review pile a person could actually work through.
///
/// Refusing to propose past this costs nothing: the memories are untouched
/// either way, and the next pass re-derives the same candidates from the same
/// store.
fn default_scheduled_max_retirement_candidates() -> usize {
    100
}

/// Five minutes.
///
/// Long enough for a large store over TCP, short enough that a pathological run
/// -- a store that grew unexpectedly, a slow socket -- is bounded rather than
/// left going.
fn default_scheduled_max_seconds() -> u64 {
    300
}

impl Default for GardenConfig {
    fn default() -> Self {
        Self {
            auto_consolidate: true,
            min_hours_between_runs: 24,
            dedup_similarity_threshold: 0.92,
            semantic_dedup_max_distance: default_semantic_dedup_max_distance(),
            semantic_review_max_distance: default_semantic_review_max_distance(),
            auto_adjudicate_review_band: false,
            auto_adjudicate_min_pairs: default_auto_adjudicate_min_pairs(),
            staleness_days: 30,
            staleness_importance_floor: 0.3,
            importance_decay_per_day: 0.01,
            max_memories: 5000,
            briefing_limit: 15,
            scheduled_consolidation: false,
            scheduled_interval_hours: default_scheduled_interval_hours(),
            scheduled_max_reversible_ops: default_scheduled_max_reversible_ops(),
            scheduled_max_retirement_candidates: default_scheduled_max_retirement_candidates(),
            scheduled_max_seconds: default_scheduled_max_seconds(),
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
