//! Pin lineage proof for consumed remediation reports.

use super::*;

/// A consumed ledger is reusable only while the live pin still proves the
/// ledger's check repairs. A whole-set publication can reset republish
/// lineage; in that case the records cannot distinguish a repaired entry
/// from an older version, so resume sends the named entries through the
/// ordinary judge-and-host-proof republish path again.
pub(super) fn consumed_ledger_findings(
    project_root: &Path,
    task_root: &Path,
    ledger: &Ledger,
) -> Result<std::collections::BTreeMap<String, String>> {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project_root, task_root);
    let pin = std::fs::read(&pin_path).ok().and_then(|bytes| {
        serde_json::from_slice::<archon_workflow::task_set_contract::AcceptancePin>(&bytes).ok()
    });
    let contract_path =
        task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let contract_bytes = std::fs::read(&contract_path)?;
    let current_digest = archon_workflow::task_set_contract::content_digest(&contract_bytes);
    let current: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&contract_bytes)?;
    let Some(pin) = pin else {
        return Ok(ledger.subjects.iter().map(|id| (
            id.clone(), format!("consumed reverify ledger {} names this check, but current pin {} is missing or unreadable; the current repaired bytes cannot be proven from pin lineage or filed chain history, so the check is being re-authored", ledger.report_id, pin_path.display())
        )).collect());
    };
    let history = archon_workflow::task_set_lineage::ChainHistory::for_pin(&pin_path);
    let mut findings = std::collections::BTreeMap::new();
    let lineage_ends_at_pin = pin
        .lineage
        .last()
        .is_some_and(|link| link.to == pin.identity());
    for id in &ledger.subjects {
        let current_entry = current
            .acceptance
            .iter()
            .chain(&current.supplementary)
            .find(|entry| &entry.id == id);
        let mut proof = None;
        let mut proves_current_repair = false;
        if lineage_ends_at_pin {
            for transition in pin
                .lineage
                .iter()
                .filter(|link| link.reauthored_ids.contains(id))
            {
                let before = history
                    .get(&transition.from.acceptance_digest)
                    .ok()
                    .flatten()
                    .and_then(|bytes| {
                        serde_json::from_slice::<
                            archon_workflow::task_set_contract::AcceptanceContract,
                        >(&bytes)
                        .ok()
                    });
                let after = if transition.to.acceptance_digest == current_digest {
                    Some(current.clone())
                } else {
                    history
                        .get(&transition.to.acceptance_digest)
                        .ok()
                        .flatten()
                        .and_then(|bytes| {
                            serde_json::from_slice::<
                                archon_workflow::task_set_contract::AcceptanceContract,
                            >(&bytes)
                            .ok()
                        })
                };
                let before_entry = before.as_ref().and_then(|contract| {
                    contract
                        .acceptance
                        .iter()
                        .chain(&contract.supplementary)
                        .find(|entry| &entry.id == id)
                });
                let after_entry = after.as_ref().and_then(|contract| {
                    contract
                        .acceptance
                        .iter()
                        .chain(&contract.supplementary)
                        .find(|entry| &entry.id == id)
                });
                if before_entry
                    .zip(after_entry)
                    .is_some_and(|(before, after)| before != after)
                    && current_entry == before_entry
                {
                    proof = Some(format!(
                        "current entry {id} matches the replaced version in transition {} -> {}, while the filed successor records different bytes",
                        transition.from.acceptance_digest, transition.to.acceptance_digest
                    ));
                    break;
                }
                if pin.acceptance_digest == current_digest
                    && before_entry
                        .zip(after_entry)
                        .is_some_and(|(before, after)| before != after)
                    && current_entry == after_entry
                {
                    proves_current_repair = true;
                }
            }
        }
        if let Some(evidence) = proof {
            findings.insert(id.clone(), format!("consumed reverify ledger {} and pin lineage prove a reverted entry: {evidence}; re-authoring it through judge and host proof", ledger.report_id));
        } else if !proves_current_repair {
            findings.insert(id.clone(), format!("consumed reverify ledger {} names this check, but current contract digest {current_digest} has no matching republish lineage proving the repaired entry; the available pin/history evidence is insufficient to establish its bytes, so re-authoring it through judge and host proof", ledger.report_id));
        }
    }
    Ok(findings)
}
