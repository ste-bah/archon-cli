//! The two checks an explicit `implements:` field makes possible and inference
//! cannot.
//!
//! Decision D5 rejected inferring the requirement→task binding, on the grounds
//! that F1 is direct evidence of inferred mapping being padded. The positive
//! case for the explicit field is here: with a declared citation you can ask
//! whether it points at anything (a **phantom citation**), and whether anything
//! points at each requirement (a **decomposition gap**). Inference can do
//! neither, because inference always produces a mapping and therefore never
//! produces a gap. That is precisely the failure mode: a report that cannot
//! come back empty is not a check.
//!
//! Both directions are reported, never repaired. An unclaimed requirement is a
//! statement about the decomposition, and inventing a task to claim it would be
//! the padding again, one level up.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::requirements::Requirement;
use super::tasks::TaskBinding;

/// A task cited a requirement ID that the PRD does not define.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhantomCitation {
    pub task_id: String,
    pub source_path: String,
    /// The ID as written in the task file.
    pub cited_id: String,
}

/// Coverage in both directions between a PRD and a task set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageReport {
    /// Requirement ID → the task IDs claiming it, sorted.
    pub claimed_by: BTreeMap<String, Vec<String>>,
    /// PRD requirements no task claims. A decomposition gap.
    pub unclaimed: Vec<String>,
    /// Citations to IDs the PRD does not contain.
    pub phantom: Vec<PhantomCitation>,
    /// Requirements claimed by more than one task. Not a fault — one code
    /// change can satisfy two requirements — but it is where double-counting
    /// starts, so it is named.
    pub multiply_claimed: Vec<String>,
    pub requirements_total: usize,
    /// Distinct requirement IDs cited across all tasks, phantoms included.
    pub citations_total: usize,
}

impl CoverageReport {
    /// True when every requirement is claimed and every citation resolves.
    pub fn is_exact(&self) -> bool {
        self.unclaimed.is_empty() && self.phantom.is_empty()
    }
}

/// Cross-check declared citations against extracted requirements.
pub fn check_coverage(requirements: &[Requirement], bindings: &[TaskBinding]) -> CoverageReport {
    let known: BTreeSet<&str> = requirements.iter().map(|r| r.id.as_str()).collect();

    let mut claimed_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut phantom = Vec::new();
    let mut cited: BTreeSet<String> = BTreeSet::new();

    for binding in bindings {
        for cited_id in &binding.implements {
            cited.insert(cited_id.clone());
            if known.contains(cited_id.as_str()) {
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

    let unclaimed = requirements
        .iter()
        .filter(|r| !claimed_by.contains_key(&r.id))
        .map(|r| r.id.clone())
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
        requirements_total: requirements.len(),
        citations_total: cited.len(),
    }
}

#[cfg(test)]
mod tests;
