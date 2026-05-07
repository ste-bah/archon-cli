use std::collections::HashSet;

use chrono::Utc;
use tracing::warn;

use super::{PRUNEABLE_TYPES, get_memories_by_type};
use crate::access::MemoryTrait;
use crate::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter};

pub(super) fn phase_importance_decay(
    graph: &dyn MemoryTrait,
    decay_per_day: f64,
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
            let new_imp = (mem.importance - (days as f64 * decay_per_day)).max(0.0);
            if new_imp < mem.importance {
                if let Err(e) = graph.update_importance(&mem.id, new_imp) {
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

pub(super) fn phase_dedup(
    graph: &dyn MemoryTrait,
    similarity_threshold: f32,
) -> Result<usize, MemoryError> {
    let mut merged = 0;
    let mut deleted_ids: HashSet<String> = HashSet::new();
    for mt in &PRUNEABLE_TYPES {
        if merged >= 50 {
            break;
        }
        let memories = get_memories_by_type(graph, *mt)?;
        for i in 0..memories.len() {
            if merged >= 50 {
                break;
            }
            if deleted_ids.contains(&memories[i].id) {
                continue;
            }
            let words_i = word_set(&memories[i].content);
            for j in (i + 1)..memories.len() {
                if merged >= 50 {
                    break;
                }
                if deleted_ids.contains(&memories[j].id) {
                    continue;
                }
                let words_j = word_set(&memories[j].content);
                if jaccard(&words_i, &words_j) <= f64::from(similarity_threshold) {
                    continue;
                }
                // Keep higher importance; if tied, keep newer.
                let (survivor, victim) = if memories[i].importance > memories[j].importance
                    || (memories[i].importance == memories[j].importance
                        && memories[i].created_at >= memories[j].created_at)
                {
                    (&memories[i], &memories[j])
                } else {
                    (&memories[j], &memories[i])
                };
                // Merge tags from victim into survivor.
                let mut merged_tags: Vec<String> = survivor.tags.clone();
                for t in &victim.tags {
                    if !merged_tags.contains(t) {
                        merged_tags.push(t.clone());
                    }
                }
                if let Err(e) = graph.update_memory(&survivor.id, None, Some(&merged_tags)) {
                    warn!(id = %survivor.id, error = %e, "failed to merge tags into survivor");
                }
                if let Err(e) = graph.create_relationship(
                    &survivor.id,
                    &victim.id,
                    RelType::Supersedes,
                    None,
                    1.0,
                ) {
                    warn!(id = %survivor.id, error = %e, "failed to create supersedes relationship");
                }
                if let Err(e) = graph.delete_memory(&victim.id) {
                    warn!(id = %victim.id, error = %e, "failed to delete duplicate");
                } else {
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
                let mut merged_tags: Vec<String> = survivor.tags.clone();
                for t in &victim.tags {
                    if !merged_tags.contains(t) {
                        merged_tags.push(t.clone());
                    }
                }
                if let Err(e) =
                    graph.update_memory(&survivor.id, Some(&combined), Some(&merged_tags))
                {
                    warn!(id = %survivor.id, error = %e, "failed to merge fragment into survivor");
                }
                if let Err(e) = graph.delete_memory(&victim.id) {
                    warn!(id = %victim.id, error = %e, "failed to delete fragment");
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
