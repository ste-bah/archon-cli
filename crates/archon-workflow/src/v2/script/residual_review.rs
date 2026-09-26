//! Issue-117: a review remediation unit whose latest verifier refused the fix
//! over blocker files no task declares. The cross-owner escalation
//! (Issue-107) widens a round only into files another task owns; a blocker
//! in a file nobody declares left the unit with nothing to buy. The host
//! plans it one REVIEW round of the unit's tasks and the tasks whose own
//! text names those files, granted exactly the files it may open.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::{Value, json};

use super::super::remediation_escalation::{blocker_evidence, candidate_paths, unit_task_ids};
use super::super::residual_paths::{TaskTexts, expandable, named_files_at, owners};
use super::super::resume_drift::remediation_unit;
use super::super::{
    WorkflowV2CallRecord, WorkflowV2HostMethod, is_reusable_status, remediation_contract,
    remediation_contract_string,
};
use super::{PlannedRound, REFUSAL_CHARS, RoundKind, clip, finished, is_residual_round, round};
use crate::task_universe::WorkflowV2TaskUniverse;

/// Each review unit's latest verifier agent, where it refused.
pub(super) fn refused_units<'a>(
    records: &[&'a WorkflowV2CallRecord],
) -> Vec<&'a WorkflowV2CallRecord> {
    let mut latest: BTreeMap<String, &WorkflowV2CallRecord> = BTreeMap::new();
    for record in records.iter().copied().filter(|record| {
        record.invalidated_by.is_none()
            && remediation_contract_string(&record.call, "stage") == Some("verify")
            && record.call.method != WorkflowV2HostMethod::Checkpoint
            && remediation_contract(&record.call)
                .is_some_and(|contract| contract.get("contest").is_none())
            && !is_residual_round(&record.call)
    }) {
        let Some((unit, _)) = remediation_unit(&record.call) else {
            continue;
        };
        let entry = latest.entry(unit).or_insert(record);
        if finished(record) >= finished(entry) {
            *entry = record;
        }
    }
    latest
        .into_values()
        .filter(|record| !is_reusable_status(record.status))
        .collect()
}

/// A REVIEW round for a refused unit whose blocker files no task declares.
pub(super) fn review_round(
    refused: &WorkflowV2CallRecord,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    texts: &TaskTexts,
    ids: &BTreeSet<String>,
) -> Option<PlannedRound> {
    let contract = remediation_contract(&refused.call)?;
    let unit_key = contract.get("taskId").and_then(Value::as_str)?.to_string();
    let unit: BTreeSet<String> = unit_task_ids(contract)
        .into_iter()
        .filter(|task| ids.contains(task))
        .collect();
    let evidence = blocker_evidence(&refused.result);
    let judged = super::super::remediation_escalation::judged_commit(&refused.result);
    let mut unowned = BTreeSet::new();
    for path in evidence
        .iter()
        .flat_map(|(summary, source)| candidate_paths(summary, source.as_deref(), Some(root)))
        .filter(|path| named_files_at(path, root, judged.as_deref()).first() == Some(path))
    {
        let declared_by = owners(universe, &path, root);
        if declared_by.iter().any(|task| !unit.contains(task)) {
            // Another task's file: the cross-owner escalation's to act on.
            return None;
        }
        if declared_by.is_empty() {
            unowned.insert(path);
        }
    }
    let naming: BTreeSet<String> = unowned
        .iter()
        .flat_map(|file| texts.naming(file, root))
        .collect();
    let tasks: BTreeSet<String> = unit.union(&naming).cloned().collect();
    let files = expandable(universe, &tasks, &unowned, root);
    if unit.is_empty() || files.is_empty() {
        return None;
    }
    let refusal = json!({
        "call_id": refused.call.id,
        "summary": clip(&refused.result.summary, REFUSAL_CHARS),
        "blocker_evidence": evidence.iter().take(6).map(|(summary, source)| json!({
            "summary": clip(summary, 600), "source": source,
        })).collect::<Vec<_>>(),
        "review_prompt": clip(refused.call.options.task.as_deref().unwrap_or_default(), REFUSAL_CHARS),
    });
    Some(round(
        RoundKind::Review,
        tasks.into_iter().collect(),
        files,
        Vec::new(),
        Some(unit_key),
        Some(refusal),
    ))
}
