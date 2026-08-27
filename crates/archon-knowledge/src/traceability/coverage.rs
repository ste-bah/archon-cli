//! The two checks an explicit `implements:` field makes possible and inference
//! cannot.
//!
//! Decision D5 rejected inferring the requirement→task binding, on the grounds
//! that F1 is direct evidence of inferred mapping being padded. The positive
//! case for the explicit field is here: with a declared citation you can ask
//! whether it points at anything (a **phantom citation**), and whether anything
//! points at each PRD obligation (a **decomposition gap**). Inference can do
//! neither, because inference always produces a mapping and therefore never
//! produces a gap. That is precisely the failure mode: a report that cannot
//! come back empty is not a check.
//!
//! Both directions are reported, never repaired. An unclaimed obligation is a
//! statement about the decomposition, and inventing a task to claim it would be
//! the padding again, one level up.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::tasks::TaskBinding;

/// A task cited an obligation ID that the PRD does not define.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhantomCitation {
    pub task_id: String,
    pub source_path: String,
    /// The ID as written in the task file.
    pub cited_id: String,
}

/// Obligation coverage in both directions between a PRD and a task set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Obligation ID → the task IDs claiming it, sorted.
    pub claimed_by: BTreeMap<String, Vec<String>>,
    /// PRD obligations no task claims. A decomposition gap.
    pub unclaimed: Vec<String>,
    /// Citations to IDs the PRD does not contain.
    pub phantom: Vec<PhantomCitation>,
    /// Obligations claimed by more than one task. Not a fault — one code
    /// change can satisfy two obligations — but it is where double-counting
    /// starts, so it is named.
    pub multiply_claimed: Vec<String>,
    /// All normative obligations. The field name is retained for JSON compatibility.
    pub requirements_total: usize,
    /// Distinct obligation IDs cited across all tasks, phantoms included.
    pub citations_total: usize,
}

impl CoverageReport {
    /// True when every obligation is claimed and every citation resolves.
    pub fn is_exact(&self) -> bool {
        self.unclaimed.is_empty() && self.phantom.is_empty()
    }
}

/// Cross-check declared citations against the caller's authoritative PRD obligations.
pub fn check_coverage(known: &BTreeSet<String>, bindings: &[TaskBinding]) -> CoverageReport {
    let mut claimed_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut phantom = Vec::new();
    let mut cited: BTreeSet<String> = BTreeSet::new();

    for binding in bindings {
        for cited_id in &binding.implements {
            cited.insert(cited_id.clone());
            if known.contains(cited_id) {
                let claimants = claimed_by.entry(cited_id.clone()).or_default();
                if !claimants.contains(&binding.task_id) {
                    claimants.push(binding.task_id.clone());
                }
            } else {
                phantom.push(PhantomCitation {
                    task_id: binding.task_id.clone(),
                    source_path: binding.source_path.clone(),
                    cited_id: cited_id.clone(),
                });
            }
        }
    }

    for claimants in claimed_by.values_mut() {
        claimants.sort();
    }

    let unclaimed = known
        .iter()
        .filter(|id| !claimed_by.contains_key(*id))
        .cloned()
        .collect();

    let multiply_claimed = claimed_by
        .iter()
        .filter(|(_, claimants)| claimants.len() > 1)
        .map(|(id, _)| id.clone())
        .collect();

    CoverageReport {
        claimed_by,
        unclaimed,
        phantom,
        multiply_claimed,
        requirements_total: known.len(),
        citations_total: cited.len(),
    }
}

#[cfg(test)]
mod tests;
