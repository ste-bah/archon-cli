//! Retain deterministic lint decisions without changing operational judge stops.
use super::*;

pub(super) fn with_decided(
    mut error: anyhow::Error,
    contract: &AcceptanceContract,
    linted: &BTreeSet<String>,
    roots: &JudgeRoots,
) -> anyhow::Error {
    let entries: Vec<_> = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .filter(|entry| linted.contains(&entry.id))
        .cloned()
        .collect();
    if entries.is_empty() {
        return error;
    }
    // FreezeIncomplete keeps its typed stop and its author feedback even when
    // the caller consumes that stop directly instead of JudgeIncomplete.
    if let Some(stopped) =
        error.downcast_mut::<crate::command::workflow_freeze_budget::FreezeIncomplete>()
    {
        let path = roots
            .task_root
            .clone()
            .unwrap_or_default()
            .join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
        stopped.add_known_findings(not_judged_findings_of(&path, &entries));
    }
    if let Some(incomplete) = error.downcast_mut::<JudgeIncomplete>() {
        incomplete.decided.extend(entries);
        return error;
    }
    error
}
