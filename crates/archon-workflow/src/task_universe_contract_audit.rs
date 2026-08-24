//! Auditing declared deliverable contracts where they are written, not where
//! they are enforced.
//!
//! # The failure this exists to prevent
//!
//! A decomposition wrote a task whose declared deliverable was
//! `${PROJECT_ROOT}/…/${DATASET_ID}/${VERSION}/validation.json` with
//! `min_instances: 1`. Every binding the gate asks for, in the one syntax it
//! refuses to read — and the task also could not have produced an instance if
//! the syntax had been right, because the datasets that path indexes are
//! created by a task it `blocks`.
//!
//! Nothing caught either half. The decomposition's own mandatory gate
//! (`archon workflow lint`) rendered three sections — diamonds, edge support,
//! fusion — and never looked at a contract. The defect surfaced seventeen hours
//! into the run, on a task whose code was complete and whose eleven focused
//! tests passed, and it consumed four remediation cycles that could not have
//! fixed it.
//!
//! # Two findings, two different kinds of claim
//!
//! [`ContractFindingKind::Unsatisfiable`] is **certain**. It asks
//! [`crate::v2::deliverable_contract::contract_defect`] — the same predicate
//! the gate runs — so anything it reports is something the runtime will refuse.
//! There is no judgement in it and no false positive is possible: lint and gate
//! are the same function.
//!
//! [`ContractFindingKind::Misallocated`] is a **heuristic**, and is reported as
//! a warning naming its reasoning. See [`instance_producer_is_plausible`] for
//! what it can and cannot see.
//!
//! # Why the raw file and not the parsed contract
//!
//! [`crate::task_universe_contract_paths::normalize_shell_path_tokens`] repairs
//! `${NAME}` into `<NAME>` as the file is read, so a run is not lost to a
//! spelling mistake. That repair would blind this audit to the source defect —
//! by the time a contract reaches the parsed universe it is already correct. So
//! the shell-token check reads the file on disk, and reports the repair rather
//! than the repaired value.

use std::collections::BTreeSet;

use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};

/// What kind of claim a finding is making.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractFindingKind {
    /// The runtime gate will refuse this contract. Certain.
    Unsatisfiable,
    /// The file on disk carries shell-style tokens that were repaired at load.
    /// The run is unaffected; the source is still wrong.
    RepairedAtLoad,
    /// The task appears unable to produce the instances it promises. Heuristic.
    Misallocated,
}

impl ContractFindingKind {
    /// Whether a finding of this kind is a statement of fact about the runtime.
    ///
    /// Callers that gate on findings should gate on this and merely report the
    /// rest: a heuristic that blocks a decomposition is worse than one that
    /// warns, because the author cannot argue with an exit code.
    pub fn is_certain(self) -> bool {
        matches!(self, Self::Unsatisfiable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractFinding {
    pub kind: ContractFindingKind,
    pub task_id: String,
    pub artifact_path: String,
    pub message: String,
}

/// Audit every declared contract in a task universe.
pub fn audit_contracts(universe: &WorkflowV2TaskUniverse) -> Vec<ContractFinding> {
    let mut findings = Vec::new();
    for task in &universe.tasks {
        for raw in shell_token_paths(&task.source_path) {
            findings.push(ContractFinding {
                kind: ContractFindingKind::RepairedAtLoad,
                task_id: task.canonical_task_id.clone(),
                artifact_path: raw.clone(),
                message: format!(
                    "declared path '{raw}' uses shell-style ${{NAME}} tokens; the gate binds \
                     <NAME> and refuses ${{NAME}}. It was repaired when the file was read, so \
                     the run is unaffected, but fix the source: an unset shell variable expands \
                     to nothing and silently makes an absolute path relative."
                ),
            });
        }
        for contract in &task.deliverable_contracts {
            let value = match serde_json::to_value(contract) {
                Ok(value) => value,
                Err(_) => continue,
            };
            if let Some(defect) = crate::v2::deliverable_contract::contract_defect(&value) {
                findings.push(ContractFinding {
                    kind: ContractFindingKind::Unsatisfiable,
                    task_id: task.canonical_task_id.clone(),
                    artifact_path: contract.artifact_path.clone(),
                    message: defect,
                });
                // One defect per contract: the second is usually the first
                // restated, and a lint that says the same thing twice gets
                // skimmed.
                continue;
            }
            if !instance_producer_is_plausible(task, contract) {
                findings.push(ContractFinding {
                    kind: ContractFindingKind::Misallocated,
                    task_id: task.canonical_task_id.clone(),
                    artifact_path: contract.artifact_path.clone(),
                    message: format!(
                        "this task promises at least {} instance(s) of a templated path but \
                         declares nothing concrete anywhere beneath '{}', so it has no footing \
                         in the tree those instances live in. Instances are produced by whatever \
                         RUNS against real data — a task that only writes source files cannot \
                         own them. Move this contract to the task that executes, or declare the \
                         concrete artifact this task really leaves there.",
                        contract.min_instances,
                        instance_root(&contract.artifact_path),
                    ),
                });
            }
        }
    }
    findings
}

/// The fixed prefix of a templated path — everything before the first token.
///
/// `.archon/data/datasets/<ID>/<VERSION>/validation.json` →
/// `.archon/data/datasets/`.
fn instance_root(path: &str) -> String {
    match path.find('<') {
        Some(open) => path[..open].to_string(),
        None => path.to_string(),
    }
}

/// Whether this task plausibly produces the instances it promises.
///
/// # The rule
///
/// A templated contract with an instance floor claims the task will leave N
/// files under some root at run time. The task is credible if it *also*
/// declares at least one concrete artifact somewhere under an ancestor of that
/// root: a task that writes real files into the data tree is one that runs
/// against the data tree.
///
/// This is what separates the two candidates in the case that motivated it. The
/// task that ingests declares a concrete registry file inside the data tree, so
/// it has footing beneath the root the instances live under. The task that
/// writes the validator declares nine source files under the code tree and
/// nothing beneath the data tree at all — it builds the checker, it never runs
/// it against a real dataset.
///
/// # What it cannot see
///
/// A task that genuinely produces instances and declares no other artifact in
/// that tree is a false positive, which is why this is a warning and never a
/// refusal. It also cannot see the reverse: a task with incidental footing in
/// the tree it does not really populate is a false negative. The rule prefers
/// the false negative — a lint that blocks correct work is abandoned, and an
/// abandoned lint catches nothing at all.
fn instance_producer_is_plausible(
    task: &WorkflowV2TaskUniverseTask,
    contract: &crate::task_universe::WorkflowV2DeliverableContract,
) -> bool {
    if contract.min_instances == 0 || !contract.artifact_path.contains('<') {
        return true;
    }
    // A declared instance source is a stronger claim than any inference here:
    // the task named where the real values come from.
    if contract.instance_source_path.is_some() || contract.instance_artifact_field.is_some() {
        return true;
    }
    let root = instance_root(&contract.artifact_path);
    let ancestors = ancestor_prefixes(&root);
    task.deliverable_contracts
        .iter()
        .filter(|other| !other.artifact_path.contains('<'))
        .any(|other| {
            ancestors
                .iter()
                .any(|ancestor| other.artifact_path.starts_with(ancestor.as_str()))
        })
}

/// Every directory prefix of a root, longest first.
///
/// `.archon/data/datasets/` yields `.archon/data/datasets/`, `.archon/data/`,
/// `.archon/`. Longest first so a caller that wants the tightest match finds it
/// before the loosest.
fn ancestor_prefixes(root: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = root.trim_end_matches('/');
    loop {
        if rest.is_empty() {
            break;
        }
        out.push(format!("{rest}/"));
        match rest.rfind('/') {
            Some(slash) => rest = &rest[..slash],
            None => break,
        }
    }
    out
}

/// Every declared contract path in a task file that still carries `${...}`.
///
/// Reads the file rather than the parsed contract because the parser repairs
/// these on the way in; see the module doc.
fn shell_token_paths(source_path: &str) -> Vec<String> {
    let Ok(contents) = std::fs::read_to_string(source_path) else {
        return Vec::new();
    };
    let mut out = BTreeSet::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed
            .strip_prefix("artifact_path:")
            .or_else(|| trimmed.strip_prefix("registry_path:"))
            .or_else(|| trimmed.strip_prefix("instance_source_path:"))
            .or_else(|| trimmed.strip_prefix("payload_path:"))
        else {
            continue;
        };
        let value = rest.trim().trim_matches(['\'', '"']);
        if value.contains("${") {
            out.insert(value.to_string());
        }
    }
    out.into_iter().collect()
}

#[cfg(test)]
#[path = "task_universe_contract_audit_tests.rs"]
mod tests;
