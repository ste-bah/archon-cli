//! Which PRD requirements the frozen acceptance checks exercise (H4, A12).
//!
//! A contract keyed only by the PRD's `AC-*` ids said nothing about the
//! PRD's requirement ids: a set could freeze eleven checks against ninety
//! requirements, pass every one of them, and finish accepted with most
//! requirements exercised by no check. So every entry declares `covers`:
//! the requirement ids whose violation, on the path it drives, makes it
//! fail. Everything here is pure and read from the PRD text and the
//! contract alone:
//!
//! - [`uncovered_requirements`]: PRD requirement ids no check covers. Each
//!   one is owed a supplementary check [`supplementary_id`] covering it, and
//!   a freeze that still has one is not finished;
//! - [`missing_acceptance_ids`]: PRD acceptance ids the contract has no
//!   check for (drift after the freeze, A7);
//! - [`tasks_without_checks`]: tasks no check answers for (A12);
//! - [`implementing_tasks`]: the tasks a check answers to, through its own
//!   id or any requirement it covers.

use std::collections::{BTreeMap, BTreeSet};

use crate::task_set_contract::{AcceptanceContract, AcceptanceCriterion};
use crate::task_universe::WorkflowV2TaskUniverse;

/// Id prefix of every supplementary check.
pub const SUPPLEMENTARY_PREFIX: &str = "SUP-";
/// Prefix of a PRD requirement id.
pub const REQUIREMENT_PREFIX: &str = "REQ-";

/// Every requirement id the PRD states, with its exact text.
pub fn prd_requirement_texts(prd: &str) -> BTreeMap<String, String> {
    crate::obligation_ids::obligation_texts(prd)
        .into_iter()
        .filter(|(id, _)| id.starts_with(REQUIREMENT_PREFIX))
        .collect()
}

/// Every requirement id the PRD states.
pub fn prd_requirement_ids(prd: &str) -> BTreeSet<String> {
    prd_requirement_texts(prd).into_keys().collect()
}

fn entries(contract: &AcceptanceContract) -> impl Iterator<Item = &AcceptanceCriterion> {
    contract.acceptance.iter().chain(&contract.supplementary)
}

/// The requirement ids some check of `contract` covers.
pub fn covered_requirements(contract: &AcceptanceContract) -> BTreeSet<String> {
    entries(contract)
        .flat_map(|entry| entry.covers.iter().map(|id| id.trim().to_string()))
        .filter(|id| !id.is_empty())
        .collect()
}

/// PRD requirement ids covered by no check: `prd_requirement_ids` less the
/// union of every entry's `covers`, sorted. Empty is the only finished state.
pub fn uncovered_requirements(
    prd_requirement_ids: &BTreeSet<String>,
    contract: &AcceptanceContract,
) -> Vec<String> {
    let covered = covered_requirements(contract);
    prd_requirement_ids
        .iter()
        .filter(|id| !covered.contains(*id))
        .cloned()
        .collect()
}

/// The supplementary check id owed to `requirement` when no check covers it.
pub fn supplementary_id(requirement: &str) -> String {
    format!("{SUPPLEMENTARY_PREFIX}{requirement}")
}

/// The requirement a supplementary id was minted for, if it was.
pub fn supplementary_requirement(id: &str) -> Option<&str> {
    id.strip_prefix(SUPPLEMENTARY_PREFIX)
        .filter(|requirement| requirement.starts_with(REQUIREMENT_PREFIX))
}

/// `(check id, covered id)` for every covered id the PRD does not define:
/// a check claiming a requirement that does not exist answers for nothing.
pub fn unknown_covers(
    prd_requirement_ids: &BTreeSet<String>,
    contract: &AcceptanceContract,
) -> Vec<(String, String)> {
    entries(contract)
        .flat_map(|entry| {
            (entry.covers.iter())
                .filter(|id| !prd_requirement_ids.contains(id.trim()))
                .map(|id| (entry.id.clone(), id.clone()))
        })
        .collect()
}

/// PRD acceptance ids the contract has no check for, sorted.
pub fn missing_acceptance_ids(
    prd_acceptance_ids: &BTreeSet<String>,
    contract: &AcceptanceContract,
) -> Vec<String> {
    let present: BTreeSet<&str> = entries(contract).map(|entry| entry.id.as_str()).collect();
    prd_acceptance_ids
        .iter()
        .filter(|id| !present.contains(id.as_str()))
        .cloned()
        .collect()
}

/// Whether `criterion` answers for a task implementing `implements`: the
/// task names the check itself, or a requirement the check covers.
pub fn answers_for(criterion: &AcceptanceCriterion, implements: &[String]) -> bool {
    implements
        .iter()
        .any(|id| id == &criterion.id || criterion.covers.iter().any(|covered| covered == id))
}

/// Every task (`(task id, implements)`) no check of `contract` answers for,
/// in input order: no frozen check can show its work done.
pub fn tasks_without_checks<'a>(
    tasks: impl IntoIterator<Item = (&'a str, &'a [String])>,
    contract: &AcceptanceContract,
) -> Vec<String> {
    tasks
        .into_iter()
        .filter(|(_, implements)| !entries(contract).any(|entry| answers_for(entry, implements)))
        .map(|(id, _)| id.to_string())
        .collect()
}

/// Tasks a check answers to, sorted: those whose `implements` names the
/// check's own id or a requirement id it covers.
pub fn implementing_tasks(
    universe: Option<&WorkflowV2TaskUniverse>,
    check_id: &str,
    covers: &[String],
) -> Vec<String> {
    universe
        .into_iter()
        .flat_map(|universe| &universe.tasks)
        .filter(|task| {
            (task.implements.iter())
                .any(|id| id == check_id || covers.iter().any(|covered| covered == id))
        })
        .map(|task| task.canonical_task_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
#[path = "acceptance_coverage_tests.rs"]
mod tests;
