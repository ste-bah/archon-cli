//! Re-authoring and publication for the selected resume remediation subjects.

use super::*;

pub(super) async fn reauthor_resume_plan(
    (project_root, prd_path, task_root, run_dir): (&Path, &Path, &Path, &Path),
    plan: &ResumePlan,
    client: &dyn archon_workflow::WorkflowLlmClient,
    cargo_limits: archon_tools::workflow_resource_env::CargoResourceLimits,
) -> Result<std::collections::BTreeMap<String, String>> {
    let contract_path =
        task_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let contract: archon_workflow::task_set_contract::AcceptanceContract = serde_json::from_slice(
        &std::fs::read(&contract_path)
            .with_context(|| format!("reading {}", contract_path.display()))?,
    )
    .with_context(|| format!("parsing {}", contract_path.display()))?;
    let check_ids = contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .map(|entry| entry.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    if let Some(pending) = &plan.pending {
        super::reconcile_pending_from_lineage(
            project_root,
            task_root,
            run_dir,
            pending,
            &contract,
        )?;
    }
    let pending_unresolved = plan
        .pending
        .as_ref()
        .map(|pending| super::unresolved_pending_subjects(run_dir, pending, &contract))
        .transpose()?;
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
        let report: serde_json::Value = serde_json::from_slice(
            &std::fs::read(&report_path)
                .with_context(|| format!("reading {}", report_path.display()))?,
        )
        .with_context(|| format!("parsing {}", report_path.display()))?;
        for id in &pending.subjects {
            if check_ids.contains(id)
                && pending_unresolved
                    .as_ref()
                    .is_some_and(|ids| ids.contains(id))
            {
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
        return Ok(std::collections::BTreeMap::new());
    }
    let scope =
        resume_author_scope(project_root, task_root, prd_path).map_err(anyhow::Error::msg)?;
    let probe =
        crate::command::workflow_task_set::executability::HostProbe::for_task_set_with_limits(
            project_root,
            task_root,
            cargo_limits,
        );
    let result = crate::command::workflow_task_set::republish::reauthor_and_republish(
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
    if let Some(pending) = &plan.pending {
        let current_bytes = std::fs::read(&contract_path)
            .with_context(|| format!("reading {}", contract_path.display()))?;
        let current: archon_workflow::task_set_contract::AcceptanceContract =
            serde_json::from_slice(&current_bytes)
                .with_context(|| format!("parsing {}", contract_path.display()))?;
        let resolved = pending
            .subjects
            .iter()
            .filter(|id| !result.unproven.contains_key(*id))
            .filter_map(|id| {
                super::entry_digest(&current, id)
                    .transpose()
                    .map(|digest| digest.map(|digest| (id.clone(), digest)))
            })
            .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
        super::append_resolved(run_dir, &pending.report_id, &resolved)?;
    }
    for (id, reason) in &result.unproven {
        eprintln!("acceptance check {id} remains pending: {reason}");
    }
    Ok(result.unproven)
}
