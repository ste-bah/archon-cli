//! Memories a background consolidation pass proposed retiring, and did not.
//!
//! The Memory Garden's pruning phases used to delete outright. That is
//! defensible when a person typed `/garden` and is reading the report; it is not
//! defensible from a timer at 3am, where the first anyone learns of it is
//! noticing that something they used to remember is gone, with nothing left to
//! say whether it was pruned, never stored, or lost to a bug.
//!
//! So a scheduled pass proposes instead. Each row here names a memory the pass
//! decided met a pruning rule and declined to act on. The memory is untouched:
//! this is a record of a *withheld* decision, not of a performed one.
//!
//! # Why a relation of its own
//!
//! `BehaviourProposal` is the general governed-learning proposal, and applying
//! one writes a new `BehaviourManifestVersion`. There is no manifest a memory
//! retirement belongs in, and no code that would turn such a version into a
//! deleted row — so routing retirements through it would produce proposals that
//! could be "applied" while nothing happened to the memory. A record that
//! reports success without doing anything is worse than no record.
//!
//! This relation says only what is true: a candidate exists, with the evidence
//! that produced it, and somebody may decide about it later. Deciding is a
//! separate act, and one nothing in the background performs.
//!
//! # Statuses
//!
//! `Pending` is the only status a background pass may write. The rest exist so a
//! decision, when a human makes one, has somewhere to land and is not lost:
//! `Approved` records consent to retire, `Rejected` records a refusal, and
//! `Applied` records that the retirement actually happened. Nothing in this
//! crate moves a row out of `Pending` on its own.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

/// Where a retirement proposal has got to.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryRetirementStatus {
    /// Proposed and awaiting a decision. The only status a background pass
    /// writes, and the state every proposal starts in.
    Pending,
    /// A person agreed the memory should be retired. Nothing has happened to it
    /// yet.
    Approved,
    /// A person declined. Kept rather than deleted, so the next pass's identical
    /// proposal can be recognised as one that was already refused.
    Rejected,
    /// The retirement was carried out.
    Applied,
}

impl MemoryRetirementStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Approved => "Approved",
            Self::Rejected => "Rejected",
            Self::Applied => "Applied",
        }
    }

    /// Parse a stored status, defaulting an unrecognised value to `Pending`.
    ///
    /// Defaulting rather than erroring, and specifically to the status that
    /// causes nothing to happen: a row written by a future version with a status
    /// this build does not know must not be readable as consent to delete.
    pub fn from_stored(value: &str) -> Self {
        match value {
            "Approved" => Self::Approved,
            "Rejected" => Self::Rejected,
            "Applied" => Self::Applied,
            _ => Self::Pending,
        }
    }
}

/// One memory a consolidation pass proposed retiring.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemoryRetirementProposalRecord {
    /// Stable across passes: derived from the memory id and the reason, so a
    /// pass that re-derives the same candidate overwrites its own earlier row
    /// rather than adding a duplicate. Without this, a nightly job proposing the
    /// same untouched memory would produce one row per night forever.
    pub proposal_id: String,
    /// The memory this is about. Still live in the memory graph.
    pub memory_id: String,
    pub memory_title: String,
    /// Enough content to recognise the memory without opening the store.
    pub excerpt: String,
    pub memory_type: String,
    pub importance: f64,
    /// `stale` or `overflow` — which rule produced this candidate.
    pub reason_kind: String,
    /// The numbers that justified it, as the pass measured them.
    pub reason_detail: String,
    /// Which consolidation pass proposed it, for joining back to its log.
    pub run_id: String,
    pub status: MemoryRetirementStatus,
    pub created_at: String,
}

impl MemoryRetirementProposalRecord {
    /// The id a given memory and reason always produce.
    ///
    /// Deterministic on purpose — see [`Self::proposal_id`].
    pub fn stable_id(memory_id: &str, reason_kind: &str) -> String {
        format!("mrp-{reason_kind}-{memory_id}")
    }
}

pub fn insert_memory_retirement_proposal(
    db: &DbInstance,
    proposal: &MemoryRetirementProposalRecord,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(proposal.proposal_id.as_str()));
    params.insert("mid".into(), DataValue::from(proposal.memory_id.as_str()));
    params.insert(
        "title".into(),
        DataValue::from(proposal.memory_title.as_str()),
    );
    params.insert("excerpt".into(), DataValue::from(proposal.excerpt.as_str()));
    params.insert(
        "mtype".into(),
        DataValue::from(proposal.memory_type.as_str()),
    );
    params.insert("importance".into(), DataValue::from(proposal.importance));
    params.insert(
        "reason".into(),
        DataValue::from(proposal.reason_kind.as_str()),
    );
    params.insert(
        "detail".into(),
        DataValue::from(proposal.reason_detail.as_str()),
    );
    params.insert("run".into(), DataValue::from(proposal.run_id.as_str()));
    params.insert("status".into(), DataValue::from(proposal.status.as_str()));
    params.insert(
        "created".into(),
        DataValue::from(proposal.created_at.as_str()),
    );

    crate::cozo_guard::run_script_guarded(
        db,
        put_script(),
        params,
        ScriptMutability::Mutable,
        "insert memory_retirement_proposals failed",
    )
    .map_err(|e| anyhow::anyhow!("insert memory_retirement_proposals failed: {e}"))?;
    Ok(())
}

pub fn get_memory_retirement_proposal(
    db: &DbInstance,
    proposal_id: &str,
) -> Result<Option<MemoryRetirementProposalRecord>> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(proposal_id));
    let result = db
        .run_script(by_id_query(), params, ScriptMutability::Immutable)
        .map_err(|e| anyhow::anyhow!("get memory_retirement_proposal failed: {e}"))?;
    Ok(result.rows.first().map(|row| row_to_record(row)))
}

/// Every proposal in a given status, newest first.
pub fn list_memory_retirement_proposals(
    db: &DbInstance,
    status: MemoryRetirementStatus,
) -> Result<Vec<MemoryRetirementProposalRecord>> {
    let mut params = BTreeMap::new();
    params.insert("status".into(), DataValue::from(status.as_str()));
    let result = db
        .run_script(by_status_query(), params, ScriptMutability::Immutable)
        .map_err(|e| anyhow::anyhow!("list memory_retirement_proposals failed: {e}"))?;
    let mut records: Vec<_> = result.rows.iter().map(|row| row_to_record(row)).collect();
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(records)
}

/// Record a human decision about a proposal.
///
/// Refuses to move a row that is not `Pending`. A decision is made once: without
/// this, a second call could quietly turn a rejection into an approval, and the
/// record of the refusal — which is the whole reason rejections are kept —
/// would be gone.
pub fn decide_memory_retirement_proposal(
    db: &DbInstance,
    proposal_id: &str,
    decision: MemoryRetirementStatus,
) -> Result<MemoryRetirementProposalRecord> {
    if decision == MemoryRetirementStatus::Pending {
        anyhow::bail!("`Pending` is the starting state, not a decision");
    }
    let Some(existing) = get_memory_retirement_proposal(db, proposal_id)? else {
        anyhow::bail!("memory retirement proposal not found: {proposal_id}");
    };
    if existing.status != MemoryRetirementStatus::Pending {
        anyhow::bail!(
            "memory retirement proposal {proposal_id} is already {}",
            existing.status.as_str()
        );
    }
    let updated = MemoryRetirementProposalRecord {
        status: decision,
        ..existing
    };
    insert_memory_retirement_proposal(db, &updated)?;
    Ok(updated)
}

fn put_script() -> &'static str {
    "?[proposal_id, memory_id, memory_title, excerpt, memory_type, importance, \
     reason_kind, reason_detail, run_id, status, created_at] <- [[$pid, $mid, \
     $title, $excerpt, $mtype, $importance, $reason, $detail, $run, $status, \
     $created]] \
     :put memory_retirement_proposals { proposal_id => memory_id, memory_title, \
     excerpt, memory_type, importance, reason_kind, reason_detail, run_id, \
     status, created_at }"
}

fn by_id_query() -> &'static str {
    "?[proposal_id, memory_id, memory_title, excerpt, memory_type, importance, \
     reason_kind, reason_detail, run_id, status, created_at] := \
     *memory_retirement_proposals{proposal_id, memory_id, memory_title, excerpt, \
     memory_type, importance, reason_kind, reason_detail, run_id, status, \
     created_at}, proposal_id = $pid"
}

fn by_status_query() -> &'static str {
    "?[proposal_id, memory_id, memory_title, excerpt, memory_type, importance, \
     reason_kind, reason_detail, run_id, status, created_at] := \
     *memory_retirement_proposals{proposal_id, memory_id, memory_title, excerpt, \
     memory_type, importance, reason_kind, reason_detail, run_id, status, \
     created_at}, status = $status"
}

fn row_to_record(row: &[DataValue]) -> MemoryRetirementProposalRecord {
    MemoryRetirementProposalRecord {
        proposal_id: str_col(row, 0).to_string(),
        memory_id: str_col(row, 1).to_string(),
        memory_title: str_col(row, 2).to_string(),
        excerpt: str_col(row, 3).to_string(),
        memory_type: str_col(row, 4).to_string(),
        importance: row[5].get_float().unwrap_or(0.0),
        reason_kind: str_col(row, 6).to_string(),
        reason_detail: str_col(row, 7).to_string(),
        run_id: str_col(row, 8).to_string(),
        status: MemoryRetirementStatus::from_stored(str_col(row, 9)),
        created_at: str_col(row, 10).to_string(),
    }
}

fn str_col(row: &[DataValue], index: usize) -> &str {
    row[index].get_str().unwrap_or("")
}

#[cfg(test)]
#[path = "memory_retirement_proposals_tests.rs"]
mod tests;
