//! Verdicts observed on a fixed pre-implementation tree.
use crate::task_set_contract::{
    AcceptanceCheck, AcceptanceContract, AcceptanceLock, content_digest,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BaselineOutcome {
    Passed,
    Failed,
}

/// Diagnostic only: the key does not authenticate live copy inputs.
/// The base snapshot must land before proof execution uses these outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineCheckResult {
    pub check_digest: String,
    pub outcome: BaselineOutcome,
}

#[derive(Debug, Clone, Default)]
pub struct BaselineResults {
    pub commit: Option<String>,
    pub outcomes: BTreeMap<String, BaselineCheckResult>,
}

impl From<Option<String>> for BaselineResults {
    fn from(commit: Option<String>) -> Self {
        Self {
            commit,
            outcomes: BTreeMap::new(),
        }
    }
}

pub fn check_digest(check: &AcceptanceCheck) -> String {
    content_digest(&serde_json::to_vec(check).expect("serializable check"))
}

pub fn results_digest(results: &BTreeMap<String, BaselineCheckResult>) -> String {
    content_digest(&serde_json::to_vec(results).expect("serializable results"))
}

impl BaselineResults {
    pub fn observed(
        commit: Option<String>,
        outcomes: &BTreeMap<String, BaselineOutcome>,
        contract: &AcceptanceContract,
    ) -> Self {
        Self {
            commit,
            outcomes: contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .filter_map(|entry| {
                    outcomes.get(&entry.id).map(|outcome| {
                        (
                            entry.id.clone(),
                            BaselineCheckResult {
                                check_digest: check_digest(&entry.check),
                                outcome: *outcome,
                            },
                        )
                    })
                })
                .collect(),
        }
    }

    pub fn retained(
        lock: &AcceptanceLock,
        commit: Option<String>,
        contract: &AcceptanceContract,
    ) -> Self {
        let mut results = Self {
            commit,
            outcomes: lock.baseline_results.clone(),
        };
        if results.commit.is_none() || results.commit != lock.baseline_commit {
            results.outcomes.clear();
        }
        results.outcomes = results.current(contract);
        results
    }

    pub fn current(&self, contract: &AcceptanceContract) -> BTreeMap<String, BaselineCheckResult> {
        contract
            .acceptance
            .iter()
            .chain(&contract.supplementary)
            .filter_map(|entry| {
                self.outcomes
                    .get(&entry.id)
                    .filter(|result| result.check_digest == check_digest(&entry.check))
                    .map(|result| (entry.id.clone(), result.clone()))
            })
            .collect()
    }
}
