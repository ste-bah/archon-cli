//! Governed proposals raised by unattended memory consolidation.
//!
//! A scheduled consolidation pass may not change anything a person would miss.
//! What it does instead is raise proposals: this memory has gone quiet, this
//! prompt rule no longer corresponds to anything being corrected, these several
//! observations say one thing and should be recorded as one claim. Each lands
//! here as a `Pending` row and waits.
//!
//! # The lifecycle is the safety property
//!
//! ```text
//!   Pending ──approve──> Approved ──apply──> Applied ──rollback──> RolledBack
//!      └─────reject────> Rejected
//! ```
//!
//! Only a human decision moves a row out of `Pending`, and only an `Approved`
//! row can be applied. A background pass writes `Pending` and nothing else, so
//! there is no sequence of automatic steps that reaches `Applied`.
//!
//! Transitions are checked here rather than at the call sites. A second caller
//! deciding an already-decided proposal is refused, so a rejection cannot be
//! quietly converted into an approval — and a rejection is precisely the record
//! that must survive, since the next pass will re-derive the same candidate and
//! needs to be recognisable as one that was already refused.
//!
//! # Why this is not a `BehaviourProposal`
//!
//! Applying a `BehaviourProposal` writes a new `BehaviourManifestVersion`. There
//! is no manifest a memory retirement belongs in and no code that would turn
//! such a version into a changed memory, so routing these through it would
//! produce proposals that report success while nothing happened. A record that
//! claims an effect it did not have is worse than no record.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

/// What a proposal asks for.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum GardenProposalKind {
    /// Withdraw a stored memory from ordinary reads.
    MemoryRetirement,
    /// Withdraw a behavioural rule from the prompt block, keeping its score.
    RuleRetirement,
    /// Record several corroborating observations as one semantic memory.
    SemanticConsolidation,
}

impl GardenProposalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MemoryRetirement => "memory_retirement",
            Self::RuleRetirement => "rule_retirement",
            Self::SemanticConsolidation => "semantic_consolidation",
        }
    }

    pub fn from_stored(value: &str) -> Option<Self> {
        match value {
            "memory_retirement" => Some(Self::MemoryRetirement),
            "rule_retirement" => Some(Self::RuleRetirement),
            "semantic_consolidation" => Some(Self::SemanticConsolidation),
            _ => None,
        }
    }
}

/// Where a proposal has got to.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum GardenProposalStatus {
    /// Raised and awaiting a decision. The only status a background pass writes.
    Pending,
    /// A person agreed. Nothing has happened to the store yet.
    Approved,
    /// A person declined. Kept, so the next pass's identical proposal is
    /// recognisable as one that was already refused.
    Rejected,
    /// The change was carried out and can be rolled back.
    Applied,
    /// An applied change was undone.
    RolledBack,
}

impl GardenProposalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Approved => "Approved",
            Self::Rejected => "Rejected",
            Self::Applied => "Applied",
            Self::RolledBack => "RolledBack",
        }
    }

    /// Parse a stored status, defaulting anything unrecognised to `Pending`.
    ///
    /// Defaulting to the status that authorises nothing, so a row written by a
    /// future build with a status this one does not know can never be read as
    /// consent to change the store.
    pub fn from_stored(value: &str) -> Self {
        match value {
            "Approved" => Self::Approved,
            "Rejected" => Self::Rejected,
            "Applied" => Self::Applied,
            "RolledBack" => Self::RolledBack,
            _ => Self::Pending,
        }
    }

    /// Whether `next` may follow this status.
    ///
    /// The whole state machine in one place, so a new call site cannot invent a
    /// transition by writing a row directly.
    pub fn may_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Pending, Self::Approved)
                | (Self::Pending, Self::Rejected)
                | (Self::Approved, Self::Applied)
                | (Self::Applied, Self::RolledBack)
        )
    }
}

/// One governed proposal.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct GardenProposalRecord {
    /// Stable across passes, so a nightly job re-deriving the same candidate
    /// re-proposes rather than accumulating a row per night.
    pub proposal_id: String,
    pub proposal_kind: GardenProposalKind,
    /// What the proposal is about: a memory id, a rule id, or a consolidation
    /// candidate id.
    pub subject_id: String,
    pub subject_title: String,
    /// Enough content to recognise the subject without opening the store.
    pub excerpt: String,
    /// The evidence, as one readable line.
    pub detail: String,
    /// Kind-specific structured data, for the applier to act on.
    pub payload_json: String,
    /// Which consolidation pass raised it.
    pub run_id: String,
    pub status: GardenProposalStatus,
    /// What the applier created or changed, so a rollback knows its target.
    pub applied_ref: String,
    pub created_at: String,
    pub decided_at: String,
}

impl GardenProposalRecord {
    /// The id a given kind and subject always produce.
    pub fn stable_id(kind: GardenProposalKind, subject_id: &str) -> String {
        format!("gp-{}-{subject_id}", kind.as_str())
    }
}

pub fn insert_garden_proposal(db: &DbInstance, proposal: &GardenProposalRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(proposal.proposal_id.as_str()));
    params.insert(
        "kind".into(),
        DataValue::from(proposal.proposal_kind.as_str()),
    );
    params.insert("sid".into(), DataValue::from(proposal.subject_id.as_str()));
    params.insert(
        "title".into(),
        DataValue::from(proposal.subject_title.as_str()),
    );
    params.insert("excerpt".into(), DataValue::from(proposal.excerpt.as_str()));
    params.insert("detail".into(), DataValue::from(proposal.detail.as_str()));
    params.insert(
        "payload".into(),
        DataValue::from(proposal.payload_json.as_str()),
    );
    params.insert("run".into(), DataValue::from(proposal.run_id.as_str()));
    params.insert("status".into(), DataValue::from(proposal.status.as_str()));
    params.insert(
        "applied".into(),
        DataValue::from(proposal.applied_ref.as_str()),
    );
    params.insert(
        "created".into(),
        DataValue::from(proposal.created_at.as_str()),
    );
    params.insert(
        "decided".into(),
        DataValue::from(proposal.decided_at.as_str()),
    );

    crate::cozo_guard::run_script_guarded(
        db,
        put_script(),
        params,
        ScriptMutability::Mutable,
        "insert garden_governed_proposals failed",
    )
    .map_err(|e| anyhow::anyhow!("insert garden_governed_proposals failed: {e}"))?;
    Ok(())
}

/// Raise a proposal, leaving an existing decision alone.
///
/// A background pass re-derives the same candidates every night. If that
/// overwrote the row, a proposal someone rejected last week would silently
/// return to `Pending` and be offered again — and a reviewer who declines the
/// same thing seven times learns to approve it to make it stop.
pub fn raise_garden_proposal(
    db: &DbInstance,
    proposal: &GardenProposalRecord,
) -> Result<GardenProposalRecord> {
    if let Some(existing) = get_garden_proposal(db, &proposal.proposal_id)?
        && existing.status != GardenProposalStatus::Pending
    {
        return Ok(existing);
    }
    insert_garden_proposal(db, proposal)?;
    Ok(proposal.clone())
}

pub fn get_garden_proposal(
    db: &DbInstance,
    proposal_id: &str,
) -> Result<Option<GardenProposalRecord>> {
    let mut params = BTreeMap::new();
    params.insert("pid".into(), DataValue::from(proposal_id));
    let result = db
        .run_script(by_id_query(), params, ScriptMutability::Immutable)
        .map_err(|e| anyhow::anyhow!("get garden proposal failed: {e}"))?;
    Ok(result.rows.first().and_then(|row| row_to_record(row)))
}

/// Every proposal in a given status, newest first.
pub fn list_garden_proposals(
    db: &DbInstance,
    status: GardenProposalStatus,
) -> Result<Vec<GardenProposalRecord>> {
    let mut params = BTreeMap::new();
    params.insert("status".into(), DataValue::from(status.as_str()));
    let result = db
        .run_script(by_status_query(), params, ScriptMutability::Immutable)
        .map_err(|e| anyhow::anyhow!("list garden proposals failed: {e}"))?;
    let mut records: Vec<_> = result
        .rows
        .iter()
        .filter_map(|row| row_to_record(row))
        .collect();
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(records)
}

/// Move a proposal to `next`, refusing any transition the lifecycle forbids.
///
/// `applied_ref` records what the applier created or changed; it is ignored for
/// transitions that change nothing in the store.
pub fn transition_garden_proposal(
    db: &DbInstance,
    proposal_id: &str,
    next: GardenProposalStatus,
    applied_ref: Option<&str>,
    at: &str,
) -> Result<GardenProposalRecord> {
    let Some(existing) = get_garden_proposal(db, proposal_id)? else {
        anyhow::bail!("garden proposal not found: {proposal_id}");
    };
    if !existing.status.may_transition_to(next) {
        anyhow::bail!(
            "garden proposal {proposal_id} is {} and cannot become {}",
            existing.status.as_str(),
            next.as_str()
        );
    }
    let updated = GardenProposalRecord {
        status: next,
        applied_ref: applied_ref
            .map(str::to_string)
            .unwrap_or(existing.applied_ref.clone()),
        decided_at: at.to_string(),
        ..existing
    };
    insert_garden_proposal(db, &updated)?;
    Ok(updated)
}

fn put_script() -> &'static str {
    "?[proposal_id, proposal_kind, subject_id, subject_title, excerpt, detail, \
     payload_json, run_id, status, applied_ref, created_at, decided_at] <- \
     [[$pid, $kind, $sid, $title, $excerpt, $detail, $payload, $run, $status, \
     $applied, $created, $decided]] \
     :put garden_governed_proposals { proposal_id => proposal_kind, subject_id, \
     subject_title, excerpt, detail, payload_json, run_id, status, applied_ref, \
     created_at, decided_at }"
}

fn by_id_query() -> &'static str {
    "?[proposal_id, proposal_kind, subject_id, subject_title, excerpt, detail, \
     payload_json, run_id, status, applied_ref, created_at, decided_at] := \
     *garden_governed_proposals{proposal_id, proposal_kind, subject_id, \
     subject_title, excerpt, detail, payload_json, run_id, status, applied_ref, \
     created_at, decided_at}, proposal_id = $pid"
}

fn by_status_query() -> &'static str {
    "?[proposal_id, proposal_kind, subject_id, subject_title, excerpt, detail, \
     payload_json, run_id, status, applied_ref, created_at, decided_at] := \
     *garden_governed_proposals{proposal_id, proposal_kind, subject_id, \
     subject_title, excerpt, detail, payload_json, run_id, status, applied_ref, \
     created_at, decided_at}, status = $status"
}

/// `None` for a row whose kind this build does not recognise.
///
/// Skipped rather than defaulted. A kind decides which applier acts on the row,
/// and guessing would point one applier at another's subject.
fn row_to_record(row: &[DataValue]) -> Option<GardenProposalRecord> {
    Some(GardenProposalRecord {
        proposal_id: str_col(row, 0).to_string(),
        proposal_kind: GardenProposalKind::from_stored(str_col(row, 1))?,
        subject_id: str_col(row, 2).to_string(),
        subject_title: str_col(row, 3).to_string(),
        excerpt: str_col(row, 4).to_string(),
        detail: str_col(row, 5).to_string(),
        payload_json: str_col(row, 6).to_string(),
        run_id: str_col(row, 7).to_string(),
        status: GardenProposalStatus::from_stored(str_col(row, 8)),
        applied_ref: str_col(row, 9).to_string(),
        created_at: str_col(row, 10).to_string(),
        decided_at: str_col(row, 11).to_string(),
    })
}

fn str_col(row: &[DataValue], index: usize) -> &str {
    row.get(index).and_then(DataValue::get_str).unwrap_or("")
}

#[cfg(test)]
#[path = "garden_proposals_tests.rs"]
mod tests;
