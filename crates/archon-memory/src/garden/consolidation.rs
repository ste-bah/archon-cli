//! Semantic-memory proposals: promoting corroborated observations to one claim.
//!
//! A cluster of provenance-compatible memories that all say the same thing is
//! evidence that the thing is settled. What consolidation proposes is to record
//! that: one semantic memory carrying the claim and the fact that several
//! independent observations support it.
//!
//! # The proposed text is a source's text, never invented prose
//!
//! The obvious design asks a model to summarise the cluster. That is exactly how
//! consolidation invents a memory no source supports — a summariser handed five
//! partially-overlapping statements will produce a sixth that generalises past
//! all of them, and the generalisation is then stored as settled knowledge with
//! the authority of five corroborating sources behind it.
//!
//! So the proposed content is verbatim the cluster's representative member. The
//! new information in a semantic memory is not the words: it is the
//! corroboration count and the raised importance. Nothing is asserted that a
//! source did not already assert, which makes the proposal reviewable against
//! its sources rather than against a reviewer's memory of them.
//!
//! # Nothing here writes
//!
//! Candidate generation takes already-read `&[Memory]` and returns values. There
//! is no store handle in this module, so the generative half of consolidation
//! provably cannot mutate anything — the same guarantee, by the same means, as
//! the pure rule-retirement analysis next door.

use serde::{Deserialize, Serialize};

use super::provenance::{DERIVED_TAG, MemoryProvenance, compatible_clusters};
use crate::types::{Memory, MemoryType};

/// Longest excerpt carried per source on a candidate.
const SOURCE_EXCERPT_MAX_CHARS: usize = 200;

/// How far above its sources a consolidated memory's importance may sit.
///
/// Corroboration justifies some elevation — several independent records of one
/// claim is better evidence than one — but not unbounded elevation, or a large
/// cluster would mint a memory that outranks everything a person wrote by hand.
/// The derived importance is the best source's, plus this, clamped to it.
const MAX_CORROBORATION_BONUS: f64 = 0.2;

/// One source behind a proposed semantic memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsolidationSource {
    pub memory_id: String,
    pub excerpt: String,
    pub importance: f64,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// A proposed semantic memory, with the sources that justify it.
///
/// Returned, never written. The memories it names are untouched; applying the
/// proposal is a separate, governed act.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticConsolidationCandidate {
    /// Stable across passes, so a nightly job re-deriving the same cluster
    /// re-proposes the same candidate rather than adding a new one each night.
    /// Derived from the sorted source ids, so a cluster that gains or loses a
    /// member is a different proposal — which it is.
    pub candidate_id: String,
    /// Verbatim content of the representative source. Not a summary.
    pub proposed_content: String,
    pub proposed_title: String,
    pub memory_type: MemoryType,
    pub project_path: String,
    pub source_type: String,
    pub proposed_importance: f64,
    /// The memory whose text the proposal carries.
    pub representative_id: String,
    pub sources: Vec<ConsolidationSource>,
}

impl SemanticConsolidationCandidate {
    pub fn corroboration_count(&self) -> usize {
        self.sources.len()
    }
}

/// Build semantic-memory proposals from provenance-compatible clusters.
///
/// `min_cluster_size` is how many independent observations must agree before a
/// claim is treated as settled; `max_span` bounds how far apart in time they may
/// be. Both are passed rather than fixed here so the policy lives with the
/// configuration that states it.
///
/// A cluster is only proposed when its members genuinely restate one another —
/// see [`cluster_is_restatement`]. Provenance compatibility says two memories
/// MAY be treated as one claim; it does not say they ARE one.
pub fn semantic_consolidation_candidates(
    memories: &[Memory],
    min_cluster_size: usize,
    max_span: chrono::Duration,
    min_word_overlap: f64,
) -> Vec<SemanticConsolidationCandidate> {
    compatible_clusters(memories, min_cluster_size, max_span)
        .into_iter()
        .filter(|cluster| cluster_is_restatement(memories, cluster, min_word_overlap))
        .filter_map(|cluster| build_candidate(memories, &cluster))
        .collect()
}

/// Whether every pair in a cluster shares enough vocabulary to be one claim.
///
/// Provenance compatibility is necessary and not sufficient. Two facts recorded
/// by one writer, in one project, on one day are provenance-compatible and may
/// be about completely different things; consolidating them would produce a
/// memory claiming five sources for a statement four of them never made.
///
/// Pairwise again rather than against the representative only, for the reason
/// the cluster itself is a clique: checking each member against one anchor lets
/// a cluster fan out around it, with members that share nothing with each other.
fn cluster_is_restatement(memories: &[Memory], cluster: &[usize], min_overlap: f64) -> bool {
    let word_sets: Vec<std::collections::HashSet<String>> = cluster
        .iter()
        .map(|&index| word_set(&memories[index].content))
        .collect();
    for (position, left) in word_sets.iter().enumerate() {
        for right in &word_sets[position + 1..] {
            if jaccard(left, right) < min_overlap {
                return false;
            }
        }
    }
    true
}

fn build_candidate(
    memories: &[Memory],
    cluster: &[usize],
) -> Option<SemanticConsolidationCandidate> {
    let representative = *cluster.iter().max_by(|&&left, &&right| {
        memories[left]
            .importance
            .total_cmp(&memories[right].importance)
            .then_with(|| memories[left].created_at.cmp(&memories[right].created_at))
    })?;
    let representative = &memories[representative];
    let provenance = MemoryProvenance::of(representative);

    let mut source_ids: Vec<&str> = cluster
        .iter()
        .map(|&index| memories[index].id.as_str())
        .collect();
    source_ids.sort_unstable();

    let best_importance = cluster
        .iter()
        .map(|&index| memories[index].importance)
        .fold(f64::MIN, f64::max);
    // Bounded elevation: enough to say "this is corroborated", never enough to
    // let a large cluster outrank a hand-written memory outright.
    let bonus = MAX_CORROBORATION_BONUS.min(0.05 * cluster.len() as f64);

    Some(SemanticConsolidationCandidate {
        candidate_id: format!("scc-{}", short_digest(&source_ids.join("|"))),
        proposed_content: representative.content.clone(),
        proposed_title: representative.title.clone(),
        memory_type: provenance.memory_type,
        project_path: provenance.project_path.to_string(),
        source_type: provenance.source_type.to_string(),
        proposed_importance: best_importance + bonus,
        representative_id: representative.id.clone(),
        sources: cluster
            .iter()
            .map(|&index| ConsolidationSource {
                memory_id: memories[index].id.clone(),
                excerpt: excerpt(&memories[index].content),
                importance: memories[index].importance,
                created_at: memories[index].created_at,
            })
            .collect(),
    })
}

/// Read the store's clusterable memories and propose consolidations.
///
/// The only function here that touches a store, and it only reads: the
/// candidate generation it delegates to takes `&[Memory]` and has no handle at
/// all. Kept separate from that pure core so the reading half can be given a
/// budget and the deciding half can be tested without one.
///
/// Superseded and retired rows never arrive: `search_memories` withholds them,
/// and [`super::provenance::ineligible_reason`] rejects them again if they do.
pub(super) fn phase_semantic_consolidation(
    graph: &dyn crate::access::MemoryTrait,
    min_cluster_size: usize,
    max_span: chrono::Duration,
    min_word_overlap: f64,
    proposed_for_retirement: &std::collections::HashSet<String>,
    ledger: &mut super::budget::BudgetLedger,
) -> Result<Vec<SemanticConsolidationCandidate>, crate::types::MemoryError> {
    let mut pool: Vec<Memory> = Vec::new();
    for memory_type in &super::PRUNEABLE_TYPES {
        pool.extend(super::get_memories_by_type(graph, *memory_type)?);
    }
    // A pass must not propose forgetting a memory and enshrining it in the same
    // breath. Both proposals would be true on their own evidence -- the row is
    // stale, and it does restate its neighbours -- but approving both would
    // retire the sources while promoting their content to a fresh, durable
    // memory, which is a strange way to honour a decision to let something go.
    //
    // Retirement wins because it is the narrower claim: it is about these
    // specific rows going quiet, while consolidation asserts the claim is
    // settled and worth keeping. If the reviewer declines the retirements, the
    // next pass finds the same cluster and proposes the consolidation then.
    pool.retain(|memory| !proposed_for_retirement.contains(&memory.id));
    let mut candidates =
        semantic_consolidation_candidates(&pool, min_cluster_size, max_span, min_word_overlap);
    // Charged against the proposal allowance, not the write allowance: this
    // phase writes nothing, and what it consumes is room in a reviewer's pile.
    let mut admitted = 0usize;
    candidates.retain(|_| {
        if ledger.take_proposal() {
            admitted += 1;
            true
        } else {
            false
        }
    });
    Ok(candidates)
}

/// Tags a consolidated memory carries once a proposal is applied.
///
/// [`DERIVED_TAG`] is what keeps the output of one pass out of the next pass's
/// clusters; the run tag says which pass produced it, so a reviewer can find
/// every memory a single night created.
pub fn derived_memory_tags(run_id: &str) -> Vec<String> {
    vec![
        DERIVED_TAG.to_string(),
        format!("{}{run_id}", super::provenance::DERIVED_FROM_RUN_TAG_PREFIX),
    ]
}

fn excerpt(content: &str) -> String {
    let flat: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    flat.chars().take(SOURCE_EXCERPT_MAX_CHARS).collect()
}

fn word_set(text: &str) -> std::collections::HashSet<String> {
    text.split_whitespace()
        .map(|word| {
            word.to_lowercase()
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

fn jaccard(
    left: &std::collections::HashSet<String>,
    right: &std::collections::HashSet<String>,
) -> f64 {
    let union = left.union(right).count() as f64;
    if union == 0.0 {
        return 0.0;
    }
    left.intersection(right).count() as f64 / union
}

/// Short stable digest, so a candidate id is readable and repeatable.
fn short_digest(value: &str) -> String {
    // FNV-1a. A cryptographic hash would be overkill for a local identity whose
    // only job is to be the same next time the same cluster is found.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
#[path = "consolidation_tests.rs"]
mod consolidation_tests;
