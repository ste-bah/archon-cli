//! A frozen task set's pin, the lineage of sanctioned republishes that led
//! to it, a write-once store of the chain versions those republishes
//! replaced, and the one check that decides whether a current pin was
//! reached from a run's launch pin by nothing but named, judge-accepted
//! re-authoring.
//!
//! The launch pin identity a run records is the anchor. A version of the
//! chain is only ever read back from the history store by the blake3 digest
//! a pin names, so a stored version authenticates itself: bytes whose hash
//! is not the named digest are refused, and so is an import of bytes no pin
//! the run or the chain names. What changed between the launch contract and
//! the current one is computed from the two contracts, never taken from a
//! claim.

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, AcceptanceCriterion, FreezeGateStamp,
    JudgeDecision, TASK_SKELETON_FILE, content_digest,
};
use crate::v2::PortableAcceptanceIdentityV1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptancePin {
    pub task_root: String,
    pub acceptance_digest: String,
    pub freeze_event_id: String,
    pub acceptance_gate: FreezeGateStamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton_gate: Option<FreezeGateStamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fidelity_waivers: Vec<crate::fidelity_audit::ObligationWaiver>,
    /// Every sanctioned per-check republish that led to this pin, oldest
    /// first. A whole-set freeze starts a new chain with none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lineage: Vec<PinTransition>,
    /// [`LINEAGE_RECORDING_V1`] when a lineage-recording freeze or republish
    /// wrote this pin; absent on a pin an older binary wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_recording: Option<u32>,
    /// PLAN-11: the digest of the check-source pins sidecar
    /// (`check_source_pins`) published with this pin; absent on a pin
    /// frozen before the sidecar existed. Not part of the identity: a judged
    /// re-pin of a source moves it without re-freezing the contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_sources_digest: Option<String>,
}

impl AcceptancePin {
    /// The identity a run records for this pin at launch.
    pub fn identity(&self) -> PortableAcceptanceIdentityV1 {
        PortableAcceptanceIdentityV1 {
            freeze_event_id: self.freeze_event_id.clone(),
            acceptance_digest: self.acceptance_digest.clone(),
            skeleton_digest: self.skeleton_digest.clone(),
        }
    }
}

/// One sanctioned republish: the pin it replaced, the pin it wrote, and the
/// checks it re-authored. Each link carries the digest of the link before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinTransition {
    pub from: PortableAcceptanceIdentityV1,
    pub to: PortableAcceptanceIdentityV1,
    pub reauthored_ids: BTreeSet<String>,
    pub trigger: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_link_digest: Option<String>,
}

impl PinTransition {
    /// The link that extends `lineage` from `from` to `to`.
    pub fn extending(
        lineage: &[PinTransition],
        from: PortableAcceptanceIdentityV1,
        to: PortableAcceptanceIdentityV1,
        reauthored_ids: BTreeSet<String>,
        trigger: &str,
    ) -> Self {
        Self {
            from,
            to,
            reauthored_ids,
            trigger: trigger.to_string(),
            prior_link_digest: lineage.last().map(PinTransition::digest),
        }
    }

    pub fn digest(&self) -> String {
        content_digest(&serde_json::to_vec(self).expect("a pin transition serializes"))
    }
}

/// The chain check that refused, by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainCheck {
    /// The pin moved and neither its lineage nor the history store proves
    /// how.
    UnrecordedChange,
    /// An import offered bytes no pin the run or the chain names.
    UnnamedDigest,
    /// A stored chain version no longer hashes to the digest it is filed by.
    PreimageCorrupt,
    /// The pin's recorded lineage does not link up, or does not end at it.
    LineageBroken,
    /// The current contract or skeleton on disk is not the one the pin binds.
    CurrentUnbound,
    /// schema_version, prd or gap_policy changed.
    ContractFieldChanged,
    /// A check was added, removed or reordered.
    CheckSetChanged,
    /// A check's criterion text changed.
    CriterionChanged,
    /// A check's gap_permitted changed.
    GapPermittedChanged,
    /// A check changed that no recorded republish re-authored.
    UnnamedCheckChanged,
    /// A changed check is not judge-accepted in the current contract.
    NotJudgeAccepted,
    /// A changed check, with no recorded republish naming it, still carries
    /// its launch judgment.
    JudgmentNotRenewed,
    /// The skeleton changed beyond the acceptance digest it binds.
    SkeletonChanged,
    /// The history store could not be read or written.
    HistoryUnavailable,
}

impl ChainCheck {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnrecordedChange => "unrecorded_change",
            Self::UnnamedDigest => "unnamed_digest",
            Self::PreimageCorrupt => "preimage_corrupt",
            Self::LineageBroken => "lineage_broken",
            Self::CurrentUnbound => "current_unbound",
            Self::ContractFieldChanged => "contract_field_changed",
            Self::CheckSetChanged => "check_set_changed",
            Self::CriterionChanged => "criterion_changed",
            Self::GapPermittedChanged => "gap_permitted_changed",
            Self::UnnamedCheckChanged => "unnamed_check_changed",
            Self::NotJudgeAccepted => "not_judge_accepted",
            Self::JudgmentNotRenewed => "judgment_not_renewed",
            Self::SkeletonChanged => "skeleton_changed",
            Self::HistoryUnavailable => "history_unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRefusal {
    pub check: ChainCheck,
    pub detail: String,
}

impl fmt::Display for ChainRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "chain check {} failed: {}",
            self.check.as_str(),
            self.detail
        )
    }
}

impl std::error::Error for ChainRefusal {}

fn refuse<T>(check: ChainCheck, detail: impl Into<String>) -> Result<T, ChainRefusal> {
    Err(ChainRefusal {
        check,
        detail: detail.into(),
    })
}

#[path = "task_set_lineage_history.rs"]
mod history;
pub use history::{ChainHistory, named_digests};

#[path = "task_set_lineage_launch.rs"]
mod launch;
pub use launch::{
    LINEAGE_RECORDING_V1, LaunchLineage, PIN_STORE_NAMESPACE, REAUTHOR_COMMAND, pin_store_dir,
    unrecorded_under_recording,
};

/// What the check proved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainProof {
    /// The pin is the launch pin itself.
    pub identical: bool,
    /// Recorded republishes from the launch pin to this one; `None` when the
    /// lineage does not reach back to the launch pin and the change was
    /// derived from the two contracts alone.
    pub recorded_hops: Option<usize>,
    /// Checks whose entries differ between the launch and current contract.
    pub changed_ids: BTreeSet<String>,
}

/// Prove `pin` (with the contract and skeleton under `tasks_root`) was
/// reached from `launch` by changing only judge-accepted checks — named by
/// the recorded lineage when it reaches back to the launch pin — and nothing
/// else: same checks in the same order, same criterion text and gap
/// permission, same contract fields, same skeleton but for the acceptance
/// digest it binds. The launch versions come from `history`, by digest.
/// Deriving the change from the two contracts alone, with no recorded
/// lineage, is allowed only when `launch_lineage` says the run predates
/// lineage recording.
pub fn verify_reached_from(
    launch: &PortableAcceptanceIdentityV1,
    launch_lineage: LaunchLineage,
    pin: &AcceptancePin,
    tasks_root: &Path,
    history: &ChainHistory,
) -> Result<ChainProof, ChainRefusal> {
    if pin.identity() == *launch {
        return Ok(ChainProof {
            identical: true,
            recorded_hops: Some(0),
            changed_ids: BTreeSet::new(),
        });
    }
    if let Some(refusal) = unrecorded_under_recording(launch, launch_lineage, pin) {
        return Err(refusal);
    }
    let named = recorded_ids(launch, pin)?;
    let current = read_bound(
        &tasks_root.join(ACCEPTANCE_CONTRACT_FILE),
        &pin.acceptance_digest,
    )?;
    let prior = history.get(&launch.acceptance_digest)?.ok_or_else(|| ChainRefusal {
        check: ChainCheck::UnrecordedChange,
        detail: format!(
            "pin {} differs from launch pin {}, and the launch contract {} is not in the chain history {}",
            pin.freeze_event_id,
            launch.freeze_event_id,
            launch.acceptance_digest,
            history.dir().display()
        ),
    })?;
    let changed_ids = contract_changes(
        &parse(&prior, ChainCheck::PreimageCorrupt, "launch contract")?,
        &parse(&current, ChainCheck::CurrentUnbound, "current contract")?,
        named.as_ref().map(|(_, ids)| ids),
    )?;
    skeleton_unchanged(launch, pin, tasks_root, history)?;
    Ok(ChainProof {
        identical: false,
        recorded_hops: named.map(|(hops, _)| hops),
        changed_ids,
    })
}

/// The recorded hops from `launch` to `pin` and the checks they re-authored,
/// or `None` when no link starts at the launch pin (the lineage then says
/// nothing about this run). From the launch pin on, a lineage that does not
/// hash-link, has a gap, or does not end at `pin` is refused; links before
/// the launch pin are not this run's and are not read.
fn recorded_ids(
    launch: &PortableAcceptanceIdentityV1,
    pin: &AcceptancePin,
) -> Result<Option<(usize, BTreeSet<String>)>, ChainRefusal> {
    let lineage = &pin.lineage;
    let Some(start) = lineage.iter().position(|link| link.from == *launch) else {
        return Ok(None);
    };
    for index in start + 1..lineage.len() {
        let (before, link) = (&lineage[index - 1], &lineage[index]);
        if link.prior_link_digest.as_deref() != Some(before.digest().as_str()) {
            return refuse(
                ChainCheck::LineageBroken,
                format!("lineage link {index} does not carry the digest of the link before it"),
            );
        }
        if before.to != link.from {
            return refuse(
                ChainCheck::LineageBroken,
                format!(
                    "lineage link {index} starts at {} but the link before it ended at {}",
                    link.from.freeze_event_id, before.to.freeze_event_id
                ),
            );
        }
    }
    let hops = &lineage[start..];
    if let Some(last) = hops.last()
        && last.to != pin.identity()
    {
        return refuse(
            ChainCheck::LineageBroken,
            format!(
                "the pin's last recorded transition ends at {}, not at the pin {}",
                last.to.freeze_event_id, pin.freeze_event_id
            ),
        );
    }
    let ids = hops
        .iter()
        .flat_map(|link| link.reauthored_ids.iter().cloned())
        .collect();
    Ok(Some((hops.len(), ids)))
}

fn read_bound(path: &Path, digest: &str) -> Result<Vec<u8>, ChainRefusal> {
    let bytes = std::fs::read(path).map_err(|error| ChainRefusal {
        check: ChainCheck::CurrentUnbound,
        detail: format!("{} could not be read: {error}", path.display()),
    })?;
    let actual = content_digest(&bytes);
    if actual != digest {
        return refuse(
            ChainCheck::CurrentUnbound,
            format!(
                "{} hashes to {actual}, but the pin binds {digest}",
                path.display()
            ),
        );
    }
    Ok(bytes)
}

fn parse<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    check: ChainCheck,
    label: &str,
) -> Result<T, ChainRefusal> {
    serde_json::from_slice(bytes).map_err(|error| ChainRefusal {
        check,
        detail: format!("the {label} is not valid: {error}"),
    })
}

/// The checks whose entries differ, refusing any other difference.
fn contract_changes(
    prior: &AcceptanceContract,
    current: &AcceptanceContract,
    named: Option<&BTreeSet<String>>,
) -> Result<BTreeSet<String>, ChainRefusal> {
    for (field, same) in [
        (
            "schema_version",
            prior.schema_version == current.schema_version,
        ),
        ("prd", prior.prd == current.prd),
        ("gap_policy", prior.gap_policy == current.gap_policy),
    ] {
        if !same {
            return refuse(
                ChainCheck::ContractFieldChanged,
                format!("the contract's {field} differs from the launch contract's"),
            );
        }
    }
    let mut changed = BTreeSet::new();
    for (section, before, after) in [
        ("acceptance", &prior.acceptance, &current.acceptance),
        (
            "supplementary",
            &prior.supplementary,
            &current.supplementary,
        ),
    ] {
        let ids = |entries: &[AcceptanceCriterion]| {
            entries
                .iter()
                .map(|entry| entry.id.clone())
                .collect::<Vec<_>>()
        };
        if ids(before) != ids(after) {
            return refuse(
                ChainCheck::CheckSetChanged,
                format!(
                    "the {section} checks are [{}] at launch but [{}] now",
                    ids(before).join(", "),
                    ids(after).join(", ")
                ),
            );
        }
        for (old, new) in before.iter().zip(after) {
            if old == new {
                continue;
            }
            let id = &new.id;
            if old.criterion != new.criterion {
                return refuse(
                    ChainCheck::CriterionChanged,
                    format!("check {id}'s criterion text changed"),
                );
            }
            if old.gap_permitted != new.gap_permitted {
                return refuse(
                    ChainCheck::GapPermittedChanged,
                    format!("check {id}'s gap_permitted changed"),
                );
            }
            if let Some(named) = named
                && !named.contains(id)
            {
                return refuse(
                    ChainCheck::UnnamedCheckChanged,
                    format!(
                        "check {id} changed, but the recorded republishes re-authored only [{}]",
                        named.iter().cloned().collect::<Vec<_>>().join(", ")
                    ),
                );
            }
            if new.judgment.verdict != JudgeDecision::Accepted {
                return refuse(
                    ChainCheck::NotJudgeAccepted,
                    format!("check {id} changed and its current judgment is not accepted"),
                );
            }
            // With no recorded republish to name it, a changed check must at
            // least carry a judgment of its own: a re-author is re-judged.
            if named.is_none() && old.judgment == new.judgment {
                return refuse(
                    ChainCheck::JudgmentNotRenewed,
                    format!("check {id} changed but still carries its launch judgment"),
                );
            }
            changed.insert(id.clone());
        }
    }
    Ok(changed)
}

/// The skeleton may differ from the launch skeleton only in the acceptance
/// digest it binds.
fn skeleton_unchanged(
    launch: &PortableAcceptanceIdentityV1,
    pin: &AcceptancePin,
    tasks_root: &Path,
    history: &ChainHistory,
) -> Result<(), ChainRefusal> {
    let (prior_digest, current_digest) = match (&launch.skeleton_digest, &pin.skeleton_digest) {
        (None, None) => return Ok(()),
        (Some(prior), Some(current)) => (prior, current),
        (Some(_), None) => {
            return refuse(
                ChainCheck::SkeletonChanged,
                "the launch pin binds a skeleton but the current pin binds none",
            );
        }
        (None, Some(_)) => {
            return refuse(
                ChainCheck::SkeletonChanged,
                "the launch pin binds no skeleton but the current pin binds one",
            );
        }
    };
    let current = read_bound(&tasks_root.join(TASK_SKELETON_FILE), current_digest)?;
    let prior = history.get(prior_digest)?.ok_or_else(|| ChainRefusal {
        check: ChainCheck::UnrecordedChange,
        detail: format!(
            "the launch skeleton {prior_digest} is not in the chain history {}",
            history.dir().display()
        ),
    })?;
    let mut prior: serde_json::Value =
        parse(&prior, ChainCheck::PreimageCorrupt, "launch skeleton")?;
    let mut current: serde_json::Value =
        parse(&current, ChainCheck::CurrentUnbound, "current skeleton")?;
    for (value, digest, label) in [
        (&mut prior, &launch.acceptance_digest, "launch"),
        (&mut current, &pin.acceptance_digest, "current"),
    ] {
        match value.get_mut("acceptance_digest") {
            Some(bound) if bound.as_str() == Some(digest.as_str()) => {
                *bound = serde_json::Value::Null
            }
            _ => {
                return refuse(
                    ChainCheck::SkeletonChanged,
                    format!("the {label} skeleton does not bind the {label} contract {digest}"),
                );
            }
        }
    }
    if prior != current {
        return refuse(
            ChainCheck::SkeletonChanged,
            "the skeleton differs from the launch skeleton beyond the acceptance digest it binds",
        );
    }
    Ok(())
}

#[cfg(test)]
#[path = "task_set_lineage_tests.rs"]
mod tests;
