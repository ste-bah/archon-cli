//! Pin lineage proof for consumed remediation reports.

use super::*;

fn consumed_record_time(project_root: &Path, report_id: &str) -> Option<std::time::SystemTime> {
    let workflows = project_root.join(".archon/workflows");
    for workflow in std::fs::read_dir(workflows).ok()? {
        let consumed = workflow
            .ok()?
            .path()
            .join("decomposition/reverify/consumed.jsonl");
        let text = match std::fs::read_to_string(&consumed) {
            Ok(text) => text,
            Err(_) => continue,
        };
        for row in text
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        {
            if row["report_id"].as_str() == Some(report_id) {
                let recorded =
                    chrono::DateTime::parse_from_rfc3339(row["consumed_at"].as_str()?).ok()?;
                return Some(
                    std::time::SystemTime::UNIX_EPOCH
                        + std::time::Duration::new(
                            recorded.timestamp().try_into().ok()?,
                            recorded.timestamp_subsec_nanos(),
                        ),
                );
            }
        }
    }
    None
}

fn history_proves_revert(
    project_root: &Path,
    pin_path: &Path,
    report_id: &str,
    id: &str,
    current_entry: Option<&archon_workflow::task_set_contract::AcceptanceCriterion>,
) -> Option<String> {
    use archon_workflow::task_set_contract::AcceptanceContract;

    let consumed_at = consumed_record_time(project_root, report_id)?;
    let history = archon_workflow::task_set_lineage::ChainHistory::for_pin(pin_path);
    let entries = std::fs::read_dir(history.dir())
        .ok()?
        .filter_map(|item| {
            let path = item.ok()?.path();
            let digest = path.file_stem()?.to_str()?;
            let bytes = history.get(digest).ok()??;
            let recorded = path.metadata().ok()?.modified().ok()?;
            let contract = serde_json::from_slice::<AcceptanceContract>(&bytes).ok()?;
            let entry = contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .find(|entry| entry.id == id)?
                .clone();
            Some((digest.to_string(), recorded, entry))
        })
        .collect::<Vec<_>>();
    let before = entries.iter().find(|(_, recorded, entry)| {
        *recorded < consumed_at && current_entry.is_some_and(|current| current.check == entry.check)
    })?;
    let after = entries
        .iter()
        .find(|(_, recorded, entry)| *recorded > consumed_at && entry.check != before.2.check)?;
    Some(format!(
        "current entry matches filed pre-consumption chain {} and differs from post-consumption repaired chain {}",
        before.0, after.0
    ))
}

/// Re-author a consumed subject only when filed lineage positively proves a
/// repair followed by a return to the older bytes. Missing lineage is not
/// evidence that a repair was lost.
pub(super) fn consumed_ledger_findings(
    project_root: &Path,
    task_root: &Path,
    ledger: &Ledger,
) -> Result<std::collections::BTreeMap<String, String>> {
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project_root, task_root);
    let contract_path =
        task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let contract_bytes = std::fs::read(&contract_path)?;
    let current_digest = archon_workflow::task_set_contract::content_digest(&contract_bytes);
    let current: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&contract_bytes)?;
    let mut findings = std::collections::BTreeMap::new();
    for id in &ledger.subjects {
        let current_entry = current
            .acceptance
            .iter()
            .chain(&current.supplementary)
            .find(|entry| &entry.id == id);
        if let Some(evidence) = history_proves_revert(
            project_root,
            &pin_path,
            &ledger.report_id,
            id,
            current_entry,
        ) {
            findings.insert(id.clone(), format!("consumed reverify ledger {} and filed chain history prove a reverted check: {evidence}; re-authoring it through judge and host proof", ledger.report_id));
        } else {
            tracing::info!(subject = %id, ledger = %ledger.report_id, current_digest = %current_digest, "skipping consumed subject: no filed pre-consumption and post-consumption repair evidence");
        }
    }
    Ok(findings)
}
