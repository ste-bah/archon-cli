use chrono::Utc;
use tracing::warn;

use super::budget::BudgetLedger;
use super::retirement::{PrunePolicy, RetirementCandidate, RetirementReason};
use super::{PRUNEABLE_TYPES, get_memories_by_type};
use crate::access::MemoryTrait;
use crate::types::{Memory, MemoryError, MemoryType, SearchFilter};

#[path = "merging.rs"]
mod merging;

pub(super) use merging::{
    DEDUP_MERGE_BUDGET, apply_adjudicated_merges, phase_dedup, phase_fragment_merge,
    phase_semantic_dedup,
};

/// What a pruning phase did, and what it declined to do.
///
/// Two fields rather than one count because they are not alternatives that can
/// be summed: `pruned` rows are gone, `candidates` name rows that are still
/// there. Collapsing them would let a caller report "8 pruned" for a pass that
/// deleted nothing.
#[derive(Debug, Default)]
pub(super) struct PruneOutcome {
    pub(super) pruned: usize,
    pub(super) candidates: Vec<RetirementCandidate>,
}

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
///
/// REVERSIBLE. Each delta is applied against an immutable provenance id, so it
/// can be identified and, if a run must be undone, countered. This is why decay
/// is the one mutating phase an unattended pass still performs directly.
pub(super) fn phase_importance_decay(
    graph: &dyn MemoryTrait,
    decay_per_day: f64,
    run_id: &str,
    previous_run: Option<chrono::DateTime<Utc>>,
    ledger: &mut BudgetLedger,
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
                // Claimed immediately before the write and never in the middle
                // of one, so a refusal always lands between whole units. The
                // rows not reached keep the importance they had, and the next
                // run bills only the span it is responsible for.
                if !ledger.take_reversible() {
                    return Ok(count);
                }
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

/// Remove -- or propose removing -- memories left untouched below the floor.
///
/// IRREVERSIBLE under [`PrunePolicy::Delete`]: `delete_memory` destroys the row
/// and there is nothing left to restore from. That is acceptable for `/garden`,
/// which a person typed and whose report they are reading. Under
/// [`PrunePolicy::Propose`] nothing is deleted and the same rows come back as
/// [`RetirementCandidate`]s.
pub(super) fn phase_staleness_prune(
    graph: &dyn MemoryTrait,
    staleness_days: u32,
    importance_floor: f64,
    policy: PrunePolicy,
    ledger: &mut BudgetLedger,
) -> Result<PruneOutcome, MemoryError> {
    let now = Utc::now();
    let threshold = chrono::Duration::days(i64::from(staleness_days));
    let mut outcome = PruneOutcome::default();
    for mt in &PRUNEABLE_TYPES {
        let memories = get_memories_by_type(graph, *mt)?;
        for mem in memories {
            let accessed = mem.last_accessed.unwrap_or(mem.created_at);
            if (now - accessed) <= threshold || mem.importance >= importance_floor {
                continue;
            }
            let reason = RetirementReason::Stale {
                days_since_access: (now - accessed).num_days(),
                staleness_days,
                importance_floor,
            };
            if !record_or_delete(graph, &mem, reason, policy, ledger, &mut outcome) {
                return Ok(outcome);
            }
        }
    }
    Ok(outcome)
}

/// Remove -- or propose removing -- the least important rows over the cap.
///
/// Same reversibility split as [`phase_staleness_prune`], and a weaker
/// justification: nothing is wrong with these memories, they simply sorted last.
/// That is why an unattended pass proposes rather than acts.
pub(super) fn phase_overflow_prune(
    graph: &dyn MemoryTrait,
    max_memories: usize,
    policy: PrunePolicy,
    ledger: &mut BudgetLedger,
) -> Result<PruneOutcome, MemoryError> {
    let mut outcome = PruneOutcome::default();
    let total = graph.memory_count()?;
    if total <= max_memories {
        return Ok(outcome);
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
    let reason = RetirementReason::Overflow {
        max_memories,
        total_memories: total,
    };
    for mem in candidates.iter().take(to_remove) {
        if !record_or_delete(graph, mem, reason.clone(), policy, ledger, &mut outcome) {
            return Ok(outcome);
        }
    }
    Ok(outcome)
}

/// Apply one pruning decision under the pass's policy.
///
/// Returns whether the phase may continue; `false` means the budget refused and
/// the caller must stop rather than move to the next candidate. Shared by both
/// pruning phases so there is exactly ONE place in the crate where a
/// consolidation pass can reach `delete_memory`, and exactly one place the
/// policy is consulted. Two copies of this decision is how one of them
/// eventually stops checking.
fn record_or_delete(
    graph: &dyn MemoryTrait,
    memory: &Memory,
    reason: RetirementReason,
    policy: PrunePolicy,
    ledger: &mut BudgetLedger,
    outcome: &mut PruneOutcome,
) -> bool {
    if !policy.may_delete() {
        if !ledger.take_proposal() {
            return false;
        }
        outcome
            .candidates
            .push(RetirementCandidate::from_memory(memory, reason));
        return true;
    }
    if !ledger.take_deletion() {
        return false;
    }
    if let Err(e) = graph.delete_memory(&memory.id) {
        warn!(id = %memory.id, error = %e, "failed to prune memory");
    } else {
        outcome.pruned += 1;
    }
    true
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

/// Record that a pass happened, INCLUDING one that ran out of budget.
///
/// Recording unconditionally looks wrong -- a pass that did half its work
/// claiming a full run -- and is nonetheless the safe choice, because the
/// timestamp's only reader is the decay bill. `phase_importance_decay` charges
/// the shorter of "since last access" and "since the previous run"; skipping the
/// write leaves the next pass billing this pass's span a second time, which
/// compounds decay and deletes memories early. Losing some pruning to the next
/// run costs a day of tidiness. Double-charging decay costs memories.
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
