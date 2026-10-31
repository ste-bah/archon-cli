use std::collections::BTreeSet;

use archon_workflow::task_set_contract::AcceptanceContract;

/// One line per named check: its id and what the judge said about it.
pub(crate) fn not_accepted_lines(
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
) -> Vec<String> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .filter(|entry| ids.contains(&entry.id))
        .map(crate::command::workflow_task_set::judge::author_note)
        .collect()
}

/// The per-check refusal every non-accepted publication reports.
pub(crate) fn not_accepted_report(
    contract: &AcceptanceContract,
    ids: &BTreeSet<String>,
    attempts: usize,
) -> String {
    report(&not_accepted_lines(contract, ids), attempts)
}

pub(super) fn report(lines: &[String], attempts: usize) -> String {
    let after = if attempts == 0 {
        String::new()
    } else {
        format!(" after {attempts} re-author attempt(s)")
    };
    format!(
        "acceptance contract not published: {} check(s) not accepted by the judge{after}; a check the judge did not accept can never run, so nothing was written:\n  - {}",
        lines.len(),
        lines.join("\n  - ")
    )
}
