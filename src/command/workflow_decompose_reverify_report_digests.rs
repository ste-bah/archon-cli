use super::*;

pub(super) fn for_report(
    task_root: &Path,
    v2: &WorkflowV2ResultStore,
    report_id: &str,
    checks: &[acceptance::CheckResult],
) -> Result<(
    Vec<String>,
    String,
    std::collections::BTreeMap<String, String>,
)> {
    let contract_path =
        task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let bytes = std::fs::read(&contract_path)
        .with_context(|| format!("reading {}", contract_path.display()))?;
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing {}", contract_path.display()))?;
    let judged = checks
        .iter()
        .filter_map(|check| {
            contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .find(|entry| entry.id == check.check_id)
                .and_then(|entry| serde_json::to_vec(entry).ok())
                .map(|entry| (check.check_id.clone(), content_digest(&entry)))
        })
        .collect();
    let mut calls = v2
        .load_call_records()?
        .into_iter()
        .map(|record| record.call.id)
        .collect::<Vec<_>>();
    calls.push(report_id.to_string());
    calls.sort();
    Ok((calls, content_digest(&bytes), judged))
}
