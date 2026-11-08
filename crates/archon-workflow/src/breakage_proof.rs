//! Diagnostic evidence from supplied perturbations of an anchored tree.
use crate::task_set_contract::content_digest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub const BREAKAGE_SCHEMA: u32 = 1;
pub const BREAKAGE_LOGIC: u32 = 1;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MutantSet {
    pub schema: u32,
    pub base_commit: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mutants: Vec<SuppliedMutant>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SuppliedMutant {
    pub task_id: String,
    pub requirement_id: String,
    pub kind: MutantKind,
    pub patch: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Located {
    pub path: String,
    pub line: u32,
    pub class: LocatedClass,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRecord {
    pub check_id: String,
    pub check_digest: String,
    pub pre: RunOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post: Option<RunOutcome>,
    pub restored_runs: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MutantRun {
    pub check_id: String,
    pub check_digest: String,
    pub control_key: String,
    pub mutant_id: String,
    pub patch_digest: String,
    pub task_id: String,
    pub requirement_id: String,
    pub kind: MutantKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub touched: Vec<String>,
    pub outcome: RunOutcome,
    pub counted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_counted: Option<NotCounted>,
    pub evidence: Evidence,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    pub patch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequirementProof {
    pub task_id: String,
    pub id: String,
    pub state: ProofState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counted_kills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BreakageProofs {
    pub schema: u32,
    pub base_commit: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub controls: BTreeMap<String, ControlRecord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub runs: BTreeMap<String, MutantRun>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<ProofSummary>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProofSummary {
    pub acceptance_digest: String,
    pub baseline_results_digest: String,
    pub snapshot_digest: String,
    pub runs_executed: u64,
    pub runs_reused: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refusals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<RequirementProof>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutantKind {
    Behaviour,
    BodyGut,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocatedClass {
    Mutated,
    CheckNamed,
    Elsewhere,
    External,
    Unresolved,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotCounted {
    TouchesShared,
    TouchesOutsideDeliverables,
    ControlUnstable,
    ControlMismatch,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofState {
    BehaviourKill,
    BodyGutKillOnly,
    SurvivorsOnly,
    NoViableMutant,
    Unverified,
    BudgetExhausted,
    HostUnrun,
    NoCountableDeliverable,
    NoPassingCheck,
    NoMutants,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunOutcome {
    Passed {
        elapsed_ms: u64,
    },
    Failed {
        exit_code: i32,
        elapsed_ms: u64,
    },
    Killed {
        exit_code: i32,
        elapsed_ms: u64,
    },
    Survived {
        elapsed_ms: u64,
    },
    Unviable {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        located: Vec<Located>,
    },
    TimedOut {
        wall_secs: u64,
    },
    NoExitStatus,
    Unrun {
        code: String,
        detail: String,
    },
    Inapplicable {
        code: String,
        detail: String,
    },
    BudgetExhausted,
}
impl SuppliedMutant {
    pub fn patch_digest(&self) -> String {
        content_digest(self.patch.as_bytes())
    }
    pub fn mutant_id(&self) -> String {
        format!("m-{}", &self.patch_digest()[..16])
    }
}
impl ControlRecord {
    pub fn stable(&self) -> bool {
        matches!(self.pre, RunOutcome::Passed { .. })
            && matches!(self.post, Some(RunOutcome::Passed { .. }))
    }
}
#[path = "breakage_selection.rs"]
pub mod selection;
#[cfg(test)]
#[path = "breakage_proof_tests.rs"]
mod tests;
