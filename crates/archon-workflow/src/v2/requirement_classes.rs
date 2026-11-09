//! Diagnostic classification; this record grants no execution authority.
use super::acceptance_stage::coverage::answers_for;
use crate::baseline_results::{BaselineCheckResult, BaselineOutcome, check_digest};
use crate::repository_record::related_paths;
use crate::task_set_contract::AcceptanceContract;
use crate::task_skeleton::TaskSkeleton;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequirementClass {
    NoCheck,
    Missing,
    CandidateDone,
    NoBaseResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fallback {
    Missing,
    Undecided,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliverablePresence {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wiring_root: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub landing_verified: bool,
    pub artifact_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub at_base: bool,
    pub live_only: bool,
    pub shared: bool,
    pub text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementEntry {
    pub task_id: String,
    pub id: String,
    pub class: RequirementClass,
    pub reason: Option<String>,
    pub fallback: Option<Fallback>,
    pub checks: BTreeMap<String, Option<BaselineOutcome>>,
    pub deliverables: Vec<DeliverablePresence>,
    pub text_item: bool,
    pub passing_check_without_deliverable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequirementClassification {
    pub schema_version: u32,
    pub baseline_commit: Option<String>,
    pub acceptance_digest: String,
    pub baseline_results_digest: String,
    pub error: Option<ClassificationError>,
    pub entries: Vec<RequirementEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_snapshot: Option<crate::base_snapshot::BaseSnapshot>,
}

pub fn classify(
    skeleton: &TaskSkeleton,
    contract: &AcceptanceContract,
    results: &BTreeMap<String, BaselineCheckResult>,
    presence: impl Fn(&str) -> (bool, bool),
) -> Vec<RequirementEntry> {
    classify_checked(skeleton, contract, results, |path| Ok(presence(path)))
}

pub fn classify_checked(
    skeleton: &TaskSkeleton,
    contract: &AcceptanceContract,
    results: &BTreeMap<String, BaselineCheckResult>,
    presence: impl Fn(&str) -> Result<(bool, bool), String>,
) -> Vec<RequirementEntry> {
    let mut entries = Vec::new();
    for (slot, task) in skeleton.tasks.iter().enumerate() {
        for id in &task.implements {
            let checks: BTreeMap<_, _> = contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .filter(|entry| answers_for(entry, std::slice::from_ref(id)))
                .map(|entry| {
                    (
                        entry.id.clone(),
                        results
                            .get(&entry.id)
                            .filter(|result| result.check_digest == check_digest(&entry.check))
                            .map(|result| result.outcome),
                    )
                })
                .collect();
            let deliverables: Vec<_> = task
                .deliverable_contracts
                .iter()
                .filter(|d| d.implements.contains(id))
                .map(|d| {
                    let result = presence(&d.artifact_path);
                    let error = result.as_ref().err().cloned();
                    let (at_base, in_checkout) = result.unwrap_or_default();
                    DeliverablePresence {
                        wiring_root: false,
                        landing_verified: false,
                        artifact_path: d.artifact_path.clone(),
                        error,
                        at_base,
                        live_only: in_checkout && !at_base,
                        shared: skeleton
                            .tasks
                            .iter()
                            .enumerate()
                            .any(|(other_slot, other)| {
                                other_slot != slot
                                    && other.deliverable_contracts.iter().any(|other| {
                                        related_paths(&d.artifact_path, &other.artifact_path)
                                    })
                            }),
                        text: d.normalized_artifact_format().as_deref() == Some("text"),
                    }
                })
                .collect();
            let passed = checks.values().any(|v| *v == Some(BaselineOutcome::Passed));
            let failed = checks.values().any(|v| *v == Some(BaselineOutcome::Failed));
            let absent = !deliverables.is_empty() && deliverables.iter().any(|d| !d.at_base);
            let (class, reason, fallback) = if checks.is_empty() {
                (RequirementClass::NoCheck, None, None)
            } else if deliverables.iter().any(|d| d.error.is_some()) {
                (
                    RequirementClass::NoBaseResult,
                    Some("deliverable_presence_error".into()),
                    None,
                )
            } else if absent {
                (
                    RequirementClass::Missing,
                    Some(
                        if deliverables.iter().any(|d| !d.at_base && d.live_only) {
                            "deliverable_unverified_at_base"
                        } else {
                            "deliverables_absent_at_base"
                        }
                        .into(),
                    ),
                    None,
                )
            } else if passed {
                (
                    RequirementClass::CandidateDone,
                    None,
                    Some(if failed {
                        Fallback::Missing
                    } else {
                        Fallback::Undecided
                    }),
                )
            } else if checks.values().all(|v| *v == Some(BaselineOutcome::Failed)) {
                (
                    RequirementClass::Missing,
                    Some("all_checks_fail_on_base".into()),
                    None,
                )
            } else {
                (RequirementClass::NoBaseResult, None, None)
            };
            entries.push(RequirementEntry {
                task_id: task.task_id.clone(),
                id: id.clone(),
                class,
                reason,
                fallback,
                checks,
                text_item: !deliverables.is_empty() && deliverables.iter().all(|d| d.text),
                passing_check_without_deliverable: absent
                    && passed
                    && class == RequirementClass::Missing,
                deliverables,
            });
        }
    }
    entries
}

#[path = "requirement_classification_error.rs"]
mod classification_error;
pub use classification_error::ClassificationError;
