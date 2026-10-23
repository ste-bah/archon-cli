//! Re-authoring and publication for the selected resume remediation subjects.

use super::*;

pub(super) async fn reauthor_resume_plan(
    (project_root, prd_path, task_root, run_dir): (&Path, &Path, &Path, &Path),
    plan: &ResumePlan,
    client: &dyn archon_workflow::WorkflowLlmClient,
    cargo_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
) -> Result<()> {
    let contract: archon_workflow::task_set_contract::AcceptanceContract =
        serde_json::from_slice(&std::fs::read(
            task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE),
        )?)?;
    let check_ids = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| entry.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut seeds = std::collections::BTreeMap::new();
    for ledger in &plan.consumed {
        let findings = consumed_ledger_findings(task_root, run_dir, ledger)?;
        for (id, finding) in findings {
            if check_ids.contains(&id) {
                seeds.insert(id, finding);
            }
        }
    }
    if let Some(pending) = &plan.pending {
        let report_path = run_dir.join(format!(
            "decomposition/reverify/reports/{}.json",
            pending.report_id
        ));
        let report: serde_json::Value = serde_json::from_slice(&std::fs::read(report_path)?)?;
        for id in &pending.subjects {
            if check_ids.contains(id) {
                let finding = report["checks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|check| check["check_id"].as_str() == Some(id))
                    .and_then(|check| check["findings"].as_array())
                    .and_then(|items| items.first())
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("the frozen check failed on the current tree");
                seeds
                    .entry(id.clone())
                    .or_insert_with(|| finding.to_string());
            }
        }
    }
    let ids = seeds
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    if ids.is_empty() {
        return Ok(());
    }
    let scope =
        resume_author_scope(project_root, task_root, prd_path).map_err(anyhow::Error::msg)?;
    let probe =
        crate::command::workflow_task_set::executability::HostProbe::for_task_set_with_limits(
            project_root,
            task_root,
            cargo_limits,
        );
    crate::command::workflow_task_set::republish::reauthor_and_republish(
        client,
        crate::command::workflow_task_set::republish::ReauthorRequest {
            project_root,
            tasks_root: task_root,
            prd_path,
            ids: &ids,
            gate: crate::command::workflow_task_set::reauthor::ReauthorGate {
                probe: &probe,
                seeds: &seeds,
            },
            trigger: "workflow resume: reverify routed a check defect to its frozen check entry",
        },
        &scope,
    )
    .await?;
    Ok(())
}
