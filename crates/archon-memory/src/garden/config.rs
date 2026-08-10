//! Configuration for the consolidation pass, and the argument for each default.
//!
//! Split out of `garden.rs` at the 500-line gate. The seam is that nothing here
//! decides anything: it is the set of numbers the phases are handed, together
//! with why each one is that number. Everything left behind acts on them.

use serde::{Deserialize, Serialize};

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
pub(super) fn default_auto_adjudicate_min_pairs() -> usize {
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
    /// How many corroborating observations make a claim worth recording once.
    ///
    /// Two is a coincidence often enough to be a poor threshold; three is the
    /// smallest count that reads as a pattern rather than a repeat.
    #[serde(default = "default_consolidation_min_cluster_size")]
    pub consolidation_min_cluster_size: usize,
    /// How far apart in time corroborating observations may be recorded.
    ///
    /// Bounds what a cluster can span, so two records of a fact that CHANGED are
    /// not consolidated into a claim that the older one is still true.
    #[serde(default = "default_consolidation_max_span_days")]
    pub consolidation_max_span_days: i64,
    /// Word overlap every pair in a cluster must reach to count as one claim.
    ///
    /// Provenance compatibility says two memories MAY be treated as one claim.
    /// This is what says they are: without it, several unrelated facts recorded
    /// by one writer on one day would be proposed as a single consolidated
    /// memory citing all of them.
    #[serde(default = "default_consolidation_min_word_overlap")]
    pub consolidation_min_word_overlap: f64,
}

/// Three independent observations.
fn default_consolidation_min_cluster_size() -> usize {
    3
}

/// A fortnight.
///
/// Long enough that a claim restated across a couple of working weeks still
/// clusters, short enough that a fact which changed last quarter does not
/// corroborate its own replacement.
fn default_consolidation_max_span_days() -> i64 {
    14
}

/// Half the vocabulary shared, pairwise.
///
/// Well above the roughly 0.31 that measured restatements of one instruction
/// reach against a lexical duplicate threshold, because this is not a duplicate
/// test: it is the floor below which a cluster stops being one claim.
fn default_consolidation_min_word_overlap() -> f64 {
    0.5
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
            consolidation_min_cluster_size: default_consolidation_min_cluster_size(),
            consolidation_max_span_days: default_consolidation_max_span_days(),
            consolidation_min_word_overlap: default_consolidation_min_word_overlap(),
        }
    }
}
