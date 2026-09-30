//! The frozen contract is held to the PRD as it is NOW (A7, H4).
//!
//! `validate_acceptance_bundle` takes its expected ids from the contract's
//! own entries, so a check list is only ever validated against itself: a
//! PRD that gained an acceptance id after the freeze, or requirements no
//! check covers, passed every round. Here the round reads the PRD the
//! contract names and records, as round-level errors that block completion,
//! every PRD acceptance id with no check and every PRD requirement id no
//! check covers (`acceptance_coverage`), each with the check it is owed and
//! the command that authors it. The checks still run: the failures they
//! show are remediated as usual while the contract is extended.

use std::collections::BTreeSet;
use std::path::PathBuf;

use archon_workflow::task_set_contract::AcceptanceContract;
use archon_workflow::v2::acceptance_stage::coverage::{
    missing_acceptance_ids, prd_requirement_ids, supplementary_id, uncovered_requirements,
};

use super::exec::StageContext;

/// The PRD `contract` names, resolved against the project root.
pub(super) fn prd_path(context: &StageContext, contract: &AcceptanceContract) -> PathBuf {
    let path = PathBuf::from(&contract.prd.path);
    if path.is_absolute() {
        path
    } else {
        context.project.join(path)
    }
}

/// Every way the frozen contract falls short of the PRD it names, as
/// round-level errors. Empty when it covers the PRD's current ids.
pub(super) fn drift_errors(context: &StageContext, contract: &AcceptanceContract) -> Vec<String> {
    let path = prd_path(context, contract);
    let prd = match std::fs::read_to_string(&path) {
        Ok(prd) => prd,
        Err(error) => {
            return vec![format!(
                "the PRD the frozen contract names ({}) cannot be read ({error}), so the contract cannot be held to its current acceptance and requirement ids; restore it",
                path.display()
            )];
        }
    };
    let command = format!(
        "cd {} && archon workflow freeze-acceptance --tasks {} --prd {}",
        context.project.display(),
        context.task_root.display(),
        path.display()
    );
    let mut errors = Vec::new();
    let acceptance: BTreeSet<String> = archon_workflow::obligation_ids::acceptance_ids(&prd);
    let missing = missing_acceptance_ids(&acceptance, contract);
    if !missing.is_empty() {
        errors.push(format!(
            "the frozen contract has no check for PRD acceptance id(s) {} ({}); author and freeze one for each: {command}",
            missing.join(", "),
            path.display()
        ));
    }
    let uncovered = uncovered_requirements(&prd_requirement_ids(&prd), contract);
    if !uncovered.is_empty() {
        errors.push(format!(
            "no frozen check covers PRD requirement id(s) {} ({}), so no round can show them met; each is owed a supplementary check ({}) covering it: {command}",
            uncovered.join(", "),
            path.display(),
            uncovered
                .iter()
                .map(|id| supplementary_id(id))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    errors
}
