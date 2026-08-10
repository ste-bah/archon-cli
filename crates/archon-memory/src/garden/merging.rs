//! Merging: the phases that fold two memories into one.
//!
//! Split out of `phases.rs` at the 500-line gate. The seam is deliberate --
//! everything here can DESTROY information, and the invariants that keep it from
//! doing so wrongly (never fold a superseded relative, never carry the
//! superseded marker onto a survivor, never write from the review band) are
//! easier to hold in view together than scattered among decay and pruning.

use std::collections::HashSet;

use tracing::warn;

use super::super::budget::BudgetLedger;
use super::super::{PRUNEABLE_TYPES, get_memories_by_type};
use crate::access::MemoryTrait;
use crate::types::{MemoryError, RelType, SUPERSEDED_TAG};

/// Most merges one consolidation pass will perform.
///
/// A pass that has already folded this many memories stops and leaves the rest
/// for the next run rather than reshaping the whole graph in one go.
pub(crate) const DEDUP_MERGE_BUDGET: usize = 50;

/// Union of both memories' tags, minus [`SUPERSEDED_TAG`].
///
/// The marker is a STATUS, not a label, and carrying it across a merge marks the
/// survivor as superseded too -- which hides it from every read path. Found when
/// `phase_fragment_merge` folded an already-superseded memory into a live one
/// and both vanished.
fn merge_tags(survivor: &crate::types::Memory, victim: &crate::types::Memory) -> Vec<String> {
    let mut merged: Vec<String> = survivor
        .tags
        .iter()
        .filter(|t| *t != SUPERSEDED_TAG)
        .cloned()
        .collect();
    for t in &victim.tags {
        if t != SUPERSEDED_TAG && !merged.contains(t) {
            merged.push(t.clone());
        }
    }
    merged
}

/// Fold `victim` into `survivor`: carry its tags across, record the supersession,
/// and mark the victim. Returns whether the victim was successfully marked.
///
/// Shared by the lexical and semantic passes so both merge identically; only
/// the way they choose candidates differs.
fn merge_duplicate(
    graph: &dyn MemoryTrait,
    survivor: &crate::types::Memory,
    victim: &crate::types::Memory,
) -> bool {
    let merged_tags = merge_tags(survivor, victim);
    if let Err(e) = graph.update_memory(&survivor.id, None, Some(&merged_tags)) {
        warn!(id = %survivor.id, error = %e, "failed to merge tags into survivor");
    }
    if let Err(e) =
        graph.create_relationship(&survivor.id, &victim.id, RelType::Supersedes, None, 1.0)
    {
        warn!(id = %survivor.id, error = %e, "failed to create supersedes relationship");
    }
    // Marked, not deleted.
    //
    // The `Supersedes` edge above used to point at a row this function then
    // destroyed, so the provenance it recorded was unreadable and a wrong merge
    // could not be undone. Keeping the row makes consolidation reversible, which
    // is what lets the merge threshold be set from measured distances instead of
    // from fear of the one-way door.
    //
    // Superseded memories are excluded from recall, search, and listing, so this
    // is invisible to everything except someone deliberately looking.
    let mut victim_tags: Vec<String> = victim.tags.clone();
    if !victim_tags.iter().any(|t| t == SUPERSEDED_TAG) {
        victim_tags.push(SUPERSEDED_TAG.to_string());
    }
    if let Err(e) = graph.update_memory(&victim.id, None, Some(&victim_tags)) {
        warn!(id = %victim.id, error = %e, "failed to mark duplicate superseded");
        false
    } else {
        true
    }
}

/// Which of two memories survives a merge: higher importance, then newer.
fn pick_survivor<'a>(
    a: &'a crate::types::Memory,
    b: &'a crate::types::Memory,
) -> (&'a crate::types::Memory, &'a crate::types::Memory) {
    if a.importance > b.importance || (a.importance == b.importance && a.created_at >= b.created_at)
    {
        (a, b)
    } else {
        (b, a)
    }
}

/// Merge memories that mean the same thing but share little vocabulary.
///
/// The lexical pass cannot see these. Eight stored restatements of one
/// instruction -- "Deploy region: eu-west-2 only", "The user requires all
/// deploys to target eu-west-2 only", and six more -- scored around 0.31
/// Jaccard against a 0.92 threshold, because set-of-words similarity does not
/// even match "deploy" to "deploys".
///
/// Distances are BANDED, not thresholded, because the ranges overlap.
///
/// Measured on a real store (`tests/semantic_distance_calibration.rs`):
/// restatements of one instruction span 0.09-0.35, while genuinely distinct
/// claims start at 0.32. No single cut separates them. So:
///
/// * below `merge_distance` -- merged, and the loser marked superseded.
/// * up to `review_distance` -- COUNTED and otherwise untouched. Probably the
///   same subject, not provably the same claim. Nothing is written: an earlier
///   version recorded a `RelatedTo` edge here, and `phase_fragment_merge` read
///   those edges as merge candidates and hard-deleted one of each pair.
/// * beyond that -- ignored.
///
/// A store without a vector index reports `None` for the merge count rather
/// than `Some(0)`, and the lexical pass still runs. The two readings are not
/// interchangeable: `Some(0)` is a store this pass examined and found clean,
/// `None` is a pass that never happened. Every Archon process after the first
/// reads memory over TCP, so `None` is the common case, not the exotic one.
pub(crate) fn phase_semantic_dedup(
    graph: &dyn MemoryTrait,
    merge_distance: f64,
    review_distance: f64,
    merge_budget: usize,
    ledger: &mut BudgetLedger,
) -> Result<(Option<usize>, Vec<crate::garden::ReviewPair>), MemoryError> {
    let mut merged = 0usize;
    let mut review: Vec<crate::garden::ReviewPair> = Vec::new();
    let mut superseded_ids: HashSet<String> = HashSet::new();

    for mt in &PRUNEABLE_TYPES {
        if merged >= merge_budget {
            break;
        }
        let memories = get_memories_by_type(graph, *mt)?;
        let by_id: std::collections::HashMap<&str, &crate::types::Memory> =
            memories.iter().map(|m| (m.id.as_str(), m)).collect();

        for memory in &memories {
            if superseded_ids.contains(&memory.id) {
                continue;
            }
            // Vector search is a property of the STORE, not of one row, so the
            // first unavailable answer settles it for the whole pass. Bailing
            // here rather than accumulating a flag also means nothing has been
            // merged yet, so there is no partial count to throw away.
            let Some(neighbours) = graph.embedding_neighbours(&memory.id, 8)? else {
                return Ok((None, Vec::new()));
            };
            for (neighbour_id, distance) in neighbours {
                if distance > review_distance || superseded_ids.contains(&neighbour_id) {
                    continue;
                }
                // Only pair within the same type: a Rule and a Fact can be
                // near-identical in wording while meaning different things to
                // injection, which renders rules and not facts.
                let Some(neighbour) = by_id.get(neighbour_id.as_str()) else {
                    continue;
                };

                if distance > merge_distance {
                    // Review band: COUNTED, not recorded in the graph.
                    //
                    // This originally wrote a `RelatedTo` edge to mark the pair
                    // for later adjudication. That was actively harmful:
                    // `phase_fragment_merge` runs next and selects candidates
                    // via `get_related_memories`, so every "probably related,
                    // decide nothing" edge became a hard delete on the very next
                    // phase. Observed on a real store -- 13 memories destroyed
                    // by pairs this band had deliberately declined to merge.
                    //
                    // A band whose purpose is to withhold a decision must not
                    // write anything another phase treats as a decision, so it
                    // reports and mutates nothing.
                    // Recorded for the caller to judge. Deduplicated by
                    // ordered id pair, because a symmetric neighbour list
                    // reports every pair from both sides.
                    let (lo, hi) = if memory.id <= neighbour.id {
                        (&memory.id, &neighbour.id)
                    } else {
                        (&neighbour.id, &memory.id)
                    };
                    if !review.iter().any(|p| p.a_id == *lo && p.b_id == *hi) {
                        review.push(crate::garden::ReviewPair {
                            a_id: lo.clone(),
                            b_id: hi.clone(),
                            a_content: memory.content.clone(),
                            b_content: neighbour.content.clone(),
                        });
                    }
                    continue;
                }

                if merged >= merge_budget {
                    continue;
                }
                let (survivor, victim) = pick_survivor(memory, neighbour);
                if superseded_ids.contains(&survivor.id) {
                    continue;
                }
                // Claimed before the merge starts, never part-way through it.
                // `merge_duplicate` is three writes -- the survivor's tags, the
                // `Supersedes` edge, the victim's marker -- and a stop between
                // them is the one partial state this pass can leave that the
                // next run does not simply redo. Refusal returns what has
                // already been merged, plus the review band found so far, which
                // costs nothing because it writes nothing.
                if !ledger.take_reversible() {
                    return Ok((Some(merged), review));
                }
                if merge_duplicate(graph, survivor, victim) {
                    superseded_ids.insert(victim.id.clone());
                    merged += 1;
                }
            }
        }
    }

    Ok((Some(merged), review))
}

/// Merge the pairs an adjudicator judged to be the same claim.
///
/// Lives here so adjudicated merges go through exactly the same path as the
/// automatic passes -- same survivor rule, same tag handling, same supersession
/// -- rather than becoming a second, subtly different way to fold two memories
/// together.
pub(crate) fn apply_adjudicated_merges(
    graph: &dyn MemoryTrait,
    verdicts: &[(crate::garden::ReviewPair, crate::garden::Adjudication)],
) -> Result<usize, MemoryError> {
    let mut merged = 0usize;
    for (pair, verdict) in verdicts {
        if *verdict != crate::garden::Adjudication::SameClaim {
            continue;
        }
        // Re-read rather than trusting the snapshot the verdict was formed
        // against. An adjudicator round-trip is slow enough for the store to
        // have moved, and folding in a memory that has since been superseded
        // would resurrect its tags into the survivor.
        let (Ok(a), Ok(b)) = (
            graph.inspect_memory(&pair.a_id),
            graph.inspect_memory(&pair.b_id),
        ) else {
            continue;
        };
        if crate::types::is_superseded(&a.tags) || crate::types::is_superseded(&b.tags) {
            continue;
        }
        let (survivor, victim) = pick_survivor(&a, &b);
        if merge_duplicate(graph, survivor, victim) {
            merged += 1;
        }
    }
    Ok(merged)
}

pub(crate) fn phase_dedup(
    graph: &dyn MemoryTrait,
    similarity_threshold: f32,
    ledger: &mut BudgetLedger,
) -> Result<usize, MemoryError> {
    let mut merged = 0;
    let mut deleted_ids: HashSet<String> = HashSet::new();
    for mt in &PRUNEABLE_TYPES {
        if merged >= DEDUP_MERGE_BUDGET {
            break;
        }
        let memories = get_memories_by_type(graph, *mt)?;
        for i in 0..memories.len() {
            if merged >= DEDUP_MERGE_BUDGET {
                break;
            }
            if deleted_ids.contains(&memories[i].id) {
                continue;
            }
            let words_i = word_set(&memories[i].content);
            for j in (i + 1)..memories.len() {
                if merged >= DEDUP_MERGE_BUDGET {
                    break;
                }
                if deleted_ids.contains(&memories[j].id) {
                    continue;
                }
                let words_j = word_set(&memories[j].content);
                if jaccard(&words_i, &words_j) <= f64::from(similarity_threshold) {
                    continue;
                }
                let (survivor, victim) = pick_survivor(&memories[i], &memories[j]);
                // See `phase_semantic_dedup`: claimed at the unit boundary so a
                // refusal never lands inside a merge.
                if !ledger.take_reversible() {
                    return Ok(merged);
                }
                if merge_duplicate(graph, survivor, victim) {
                    deleted_ids.insert(victim.id.clone());
                    merged += 1;
                }
            }
        }
    }
    Ok(merged)
}

pub(crate) fn phase_fragment_merge(
    graph: &dyn MemoryTrait,
    ledger: &mut BudgetLedger,
) -> Result<usize, MemoryError> {
    let mut merged = 0;
    let mut deleted_ids: HashSet<String> = HashSet::new();
    for mt in &PRUNEABLE_TYPES {
        if merged >= 20 {
            break;
        }
        let memories = get_memories_by_type(graph, *mt)?;
        for mem in &memories {
            if merged >= 20 {
                break;
            }
            if deleted_ids.contains(&mem.id) {
                continue;
            }
            let related = match graph.get_related_memories(&mem.id, 1) {
                Ok(r) => r,
                Err(_) => continue,
            };
            for rel in &related {
                if merged >= 20 {
                    break;
                }
                if deleted_ids.contains(&rel.id) {
                    continue;
                }
                // A superseded relative is the losing half of a merge that
                // already happened, and the `Supersedes` edge that records it is
                // a relationship like any other to `get_related_memories`.
                // Without this, every duplicate the dedup phase folds away comes
                // straight back in the same run -- concatenated onto the
                // survivor, whose content then reads as its own text twice, in
                // the prompt, on every recall.
                if rel.tags.iter().any(|t| t == SUPERSEDED_TAG) {
                    continue;
                }
                if rel.memory_type != mem.memory_type {
                    continue;
                }
                if mem.content.len() + rel.content.len() + 3 > 500 {
                    continue;
                }
                // Keep the one with higher importance.
                let (survivor, victim) = if mem.importance >= rel.importance {
                    (mem, rel)
                } else {
                    (rel, mem)
                };
                // Claimed before the concatenation, which is the destructive
                // half: it rewrites the survivor's content. A refusal here
                // leaves both rows exactly as they were.
                if !ledger.take_reversible() {
                    return Ok(merged);
                }
                let combined = format!("{} | {}", survivor.content, victim.content);
                let merged_tags = merge_tags(survivor, victim);
                if let Err(e) =
                    graph.update_memory(&survivor.id, Some(&combined), Some(&merged_tags))
                {
                    warn!(id = %survivor.id, error = %e, "failed to merge fragment into survivor");
                }
                // Superseded, not deleted -- same reasoning as `merge_duplicate`.
                // Fragment merge concatenates content into the survivor, so the
                // text survives either way, but the row carries the id that
                // relationships and provenance point at.
                let mut victim_tags: Vec<String> = victim.tags.clone();
                if !victim_tags.iter().any(|t| t == SUPERSEDED_TAG) {
                    victim_tags.push(SUPERSEDED_TAG.to_string());
                }
                if let Err(e) = graph.update_memory(&victim.id, None, Some(&victim_tags)) {
                    warn!(id = %victim.id, error = %e, "failed to mark fragment superseded");
                } else {
                    deleted_ids.insert(victim.id.clone());
                    merged += 1;
                }
            }
        }
    }
    Ok(merged)
}

/// Build a set of normalised words from content for Jaccard comparison.
fn word_set(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|w| {
            w.to_lowercase()
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Jaccard similarity between two word sets.
fn jaccard(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let intersection = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}
