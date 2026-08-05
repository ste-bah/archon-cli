use std::collections::HashSet;

use chrono::Utc;
use tracing::warn;

use super::{PRUNEABLE_TYPES, get_memories_by_type};
use crate::access::MemoryTrait;
use crate::types::{Memory, MemoryError, MemoryType, RelType, SUPERSEDED_TAG, SearchFilter};

pub(super) fn phase_importance_decay(
    graph: &dyn MemoryTrait,
    decay_per_day: f64,
    run_id: &str,
) -> Result<usize, MemoryError> {
    let now = Utc::now();
    let mut count = 0;
    for mt in &PRUNEABLE_TYPES {
        let memories = get_memories_by_type(graph, *mt)?;
        for mem in memories {
            let accessed = mem.last_accessed.unwrap_or(mem.created_at);
            let days = (now - accessed).num_days();
            if days < 1 {
                continue;
            }
            let delta = -(days as f64 * decay_per_day).min(mem.importance);
            if delta < 0.0 {
                let provenance_id = format!("garden-decay:{run_id}:{}", mem.id);
                if let Err(e) = graph.apply_importance_delta(&mem.id, delta, &provenance_id) {
                    warn!(id = %mem.id, error = %e, "failed to decay importance");
                } else {
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}

pub(super) fn phase_staleness_prune(
    graph: &dyn MemoryTrait,
    staleness_days: u32,
    importance_floor: f64,
) -> Result<usize, MemoryError> {
    let now = Utc::now();
    let threshold = chrono::Duration::days(i64::from(staleness_days));
    let mut count = 0;
    for mt in &PRUNEABLE_TYPES {
        let memories = get_memories_by_type(graph, *mt)?;
        for mem in memories {
            let accessed = mem.last_accessed.unwrap_or(mem.created_at);
            if (now - accessed) > threshold && mem.importance < importance_floor {
                if let Err(e) = graph.delete_memory(&mem.id) {
                    warn!(id = %mem.id, error = %e, "failed to prune stale memory");
                } else {
                    count += 1;
                }
            }
        }
    }
    Ok(count)
}

/// Most merges one consolidation pass will perform.
///
/// A pass that has already folded this many memories stops and leaves the rest
/// for the next run rather than reshaping the whole graph in one go.
pub(super) const DEDUP_MERGE_BUDGET: usize = 50;

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
/// A store without a vector index returns no neighbours, and this becomes a
/// no-op rather than an error -- the lexical pass still runs.
pub(super) fn phase_semantic_dedup(
    graph: &dyn MemoryTrait,
    merge_distance: f64,
    review_distance: f64,
    merge_budget: usize,
) -> Result<(usize, usize), MemoryError> {
    let mut merged = 0usize;
    let mut linked = 0usize;
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
            let neighbours = graph.embedding_neighbours(&memory.id, 8)?;
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
                    linked += 1;
                    continue;
                }

                if merged >= merge_budget {
                    continue;
                }
                let (survivor, victim) = pick_survivor(memory, neighbour);
                if superseded_ids.contains(&survivor.id) {
                    continue;
                }
                if merge_duplicate(graph, survivor, victim) {
                    superseded_ids.insert(victim.id.clone());
                    merged += 1;
                }
            }
        }
    }

    Ok((merged, linked))
}

pub(super) fn phase_dedup(
    graph: &dyn MemoryTrait,
    similarity_threshold: f32,
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
                if merge_duplicate(graph, survivor, victim) {
                    deleted_ids.insert(victim.id.clone());
                    merged += 1;
                }
            }
        }
    }
    Ok(merged)
}

pub(super) fn phase_fragment_merge(graph: &dyn MemoryTrait) -> Result<usize, MemoryError> {
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

pub(super) fn phase_overflow_prune(
    graph: &dyn MemoryTrait,
    max_memories: usize,
) -> Result<usize, MemoryError> {
    let total = graph.memory_count()?;
    if total <= max_memories {
        return Ok(0);
    }
    let to_remove = total - max_memories;
    // Gather all pruneable memories, sort by importance ASC then created_at ASC.
    let mut candidates: Vec<Memory> = Vec::new();
    for mt in &PRUNEABLE_TYPES {
        candidates.extend(get_memories_by_type(graph, *mt)?);
    }
    candidates.sort_by(|a, b| {
        a.importance
            .partial_cmp(&b.importance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });
    let mut count = 0;
    for mem in candidates.iter().take(to_remove) {
        if let Err(e) = graph.delete_memory(&mem.id) {
            warn!(id = %mem.id, error = %e, "failed to prune overflow memory");
        } else {
            count += 1;
        }
    }
    Ok(count)
}

pub(super) fn phase_record_timestamp(graph: &dyn MemoryTrait) -> Result<(), MemoryError> {
    let now_str = Utc::now().to_rfc3339();
    let filter = SearchFilter {
        tags: vec!["garden:last_run".into()],
        require_all_tags: true,
        ..SearchFilter::default()
    };
    let results = graph.search_memories(&filter)?;
    if let Some(existing) = results.first() {
        graph.update_memory(&existing.id, Some(&now_str), None)?;
    } else {
        graph.store_memory(
            &now_str,
            "garden:last_run",
            MemoryType::Fact,
            1.0,
            &["garden:last_run".into()],
            "garden",
            "",
        )?;
    }
    Ok(())
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
