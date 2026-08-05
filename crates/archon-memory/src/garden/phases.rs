use chrono::Utc;
use tracing::warn;

use super::{PRUNEABLE_TYPES, get_memories_by_type};
use crate::access::MemoryTrait;
use crate::types::{Memory, MemoryError, MemoryType, SearchFilter};

#[path = "merging.rs"]
mod merging;

pub(super) use merging::{
    DEDUP_MERGE_BUDGET, apply_adjudicated_merges, phase_dedup, phase_fragment_merge,
    phase_semantic_dedup,
};

/// Reduce importance for memories that have gone untouched.
///
/// CHARGED PER RUN, NOT PER AGE. The obvious version bills the whole span since
/// `last_accessed` on every run, and since `last_accessed` only moves when a
/// memory is actually recalled, every run bills that same span again. Decay then
/// compounds: measured at 1.0/day on a 2-day-old memory, three consecutive
/// sessions took importance 50 -> 48 -> 46 -> 44 rather than stopping at 48.
///
/// The real-world effect at the shipped `0.01/day` is that a memory stored at
/// importance 0.5 and never recalled reaches zero in about ten days instead of
/// fifty, crosses the 0.3 staleness floor in about a week, and is deleted at
/// `staleness_days`. Anything written by hand is hit hardest, because it is
/// stored at a fixed 0.5 and is not re-accessed unless recall happens to pick it.
///
/// So the bill is the shorter of "since last accessed" and "since the previous
/// consolidation" -- the increment this run is actually responsible for. With no
/// previous run recorded, the first run catches up from creation, which is the
/// intent.
pub(super) fn phase_importance_decay(
    graph: &dyn MemoryTrait,
    decay_per_day: f64,
    run_id: &str,
    previous_run: Option<chrono::DateTime<Utc>>,
) -> Result<usize, MemoryError> {
    let now = Utc::now();
    let mut count = 0;
    for mt in &PRUNEABLE_TYPES {
        let memories = get_memories_by_type(graph, *mt)?;
        for mem in memories {
            let accessed = mem.last_accessed.unwrap_or(mem.created_at);
            let since_accessed = (now - accessed).num_days();
            let days = match previous_run {
                Some(previous) => since_accessed.min((now - previous).num_days()),
                None => since_accessed,
            };
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

/// When consolidation last ran, or `None` if it never has (or the stored value
/// is unreadable, which is treated the same as never — see
/// [`phase_importance_decay`], where that means catching up from creation).
pub(super) fn read_last_run(
    graph: &dyn MemoryTrait,
) -> Result<Option<chrono::DateTime<Utc>>, MemoryError> {
    let filter = SearchFilter {
        tags: vec!["garden:last_run".into()],
        require_all_tags: true,
        ..SearchFilter::default()
    };
    Ok(graph
        .search_memories(&filter)?
        .first()
        .and_then(|m| m.content.parse::<chrono::DateTime<Utc>>().ok()))
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
