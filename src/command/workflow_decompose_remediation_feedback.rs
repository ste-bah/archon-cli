//! Carry the observed findings into the same open-finding body author path.
use super::*;

pub(super) fn body_feedback(
    run_dir: &Path,
    arguments: &mut serde_json::Value,
    ledger: Option<&Ledger>,
    skeleton: &archon_workflow::task_skeleton::TaskSkeleton,
    contract: &archon_workflow::task_set_contract::AcceptanceContract,
) -> Result<()> {
    let Some(ledger) = ledger else {
        return Ok(());
    };
    let report = read_author_report(run_dir, &ledger.report_id)?;
    let mut feedback = std::collections::BTreeMap::new();
    for id in ledger
        .subjects
        .iter()
        .filter(|id| skeleton.tasks.iter().any(|task| &task.task_id == *id))
    {
        let mut findings = Vec::new();
        for subject in report["subjects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|subject| subject["task_id"].as_str() == Some(id))
        {
            findings.extend(
                subject["findings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|value| value.as_str().map(str::to_owned)),
            );
        }
        for gate in ["set_gates", "frozen_gates"]
            .into_iter()
            .flat_map(|key| report[key].as_array().into_iter().flatten())
        {
            if gate["subjects"]
                .as_array()
                .is_some_and(|subjects| subjects.iter().any(|subject| subject.as_str() == Some(id)))
            {
                findings.extend(
                    gate["findings"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|value| value.as_str().map(str::to_owned)),
                );
            }
        }
        // Behavioral failures are routed to implementing bodies by the ledger.
        // Attribute each check explicitly; do not claim the check text came
        // from the body judge.
        for check in report["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|check| check["remediation_route"].as_str() == Some("task"))
        {
            let entry = contract
                .acceptance
                .iter()
                .chain(&contract.supplementary)
                .find(|entry| check["check_id"].as_str() == Some(entry.id.as_str()));
            let applies = entry.is_some_and(|entry| {
                skeleton.tasks.iter().any(|task| {
                    &task.task_id == id
                        && entry
                            .covers
                            .iter()
                            .chain(std::iter::once(&entry.id))
                            .any(|obligation| task.implements.contains(obligation))
                })
            });
            if !applies {
                continue;
            }
            for finding in check["findings"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str())
            {
                findings.push(format!(
                    "Check {} reported: {finding}",
                    check["check_id"].as_str().unwrap()
                ));
            }
        }
        feedback.insert(id, deduplicate_subjects(findings));
    }
    arguments["remediationBodyFindings"] = serde_json::to_value(feedback)?;
    Ok(())
}

/// Planning selects subjects; live continuation additionally supplies the
/// report's observations to the authors that will repair them.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_with_feedback(
    store: &archon_workflow::WorkflowStore,
    run_id: &str,
    run_dir: &Path,
    arguments: &mut serde_json::Value,
    runtime: &archon_workflow::FixedRunIdentityV1,
    criteria: &std::collections::BTreeMap<String, String>,
    requirement_texts: &std::collections::BTreeMap<String, String>,
    remediate: bool,
) -> Result<Option<ResumePlan>> {
    let plan = prepare(
        store,
        run_id,
        run_dir,
        arguments,
        runtime,
        criteria,
        requirement_texts,
    )?;
    if remediate && let Some(pending) = plan.as_ref().and_then(|plan| plan.pending.as_ref()) {
        let root = Path::new(&runtime.task_root_identity);
        let skeleton_path = root.join(archon_workflow::task_set_contract::TASK_SKELETON_FILE);
        let contract_path = root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
        let skeleton = serde_json::from_slice(
            &std::fs::read(&skeleton_path)
                .with_context(|| format!("reading {}", skeleton_path.display()))?,
        )?;
        let contract = serde_json::from_slice(
            &std::fs::read(&contract_path)
                .with_context(|| format!("reading {}", contract_path.display()))?,
        )?;
        body_feedback(run_dir, arguments, Some(pending), &skeleton, &contract)?;
    }
    Ok(plan)
}

#[cfg(test)]
#[path = "workflow_decompose_remediation_feedback_tests.rs"]
mod tests;
