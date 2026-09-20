//! Reuse only unchanged accepted judgments from a verified previous freeze.
use super::*;
use archon_workflow::task_set_contract::JudgeDecision;

pub(super) async fn judge(
    project: &Path,
    tasks: &Path,
    client: &dyn WorkflowLlmClient,
    mut contract: AcceptanceContract,
    expected: &BTreeSet<String>,
) -> Result<AcceptanceContract> {
    // A candidate's own judgment fields never authorize reuse. Failed validation
    // of historical evidence means rejudge, not silently trust its contents.
    let base = std::fs::read(acceptance_pin_path(project, tasks))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AcceptancePin>(&bytes).ok())
        .and_then(|pin| validate_acceptance_bundle(tasks, Some(&pin), expected).ok());
    let mut reused = BTreeSet::new();
    if let Some(base) = &base {
        if base.prd == contract.prd && base.gap_policy == contract.gap_policy {
            let defective: BTreeSet<_> = acceptance_policy_findings(base)
                .into_iter()
                .map(|finding| finding.field.split('.').next().unwrap_or("").to_string())
                .collect();
            for entry in contract
                .acceptance
                .iter_mut()
                .chain(&mut contract.supplementary)
            {
                let Some(old) = base
                    .acceptance
                    .iter()
                    .chain(&base.supplementary)
                    .find(|old| old.id == entry.id)
                else {
                    continue;
                };
                let sampling_matches = old.judgment.sampling.as_ref().is_some_and(|sampling| {
                    sampling["model"] == serde_json::json!(client.resolve_model_alias("sonnet"))
                        && sampling["provider"] == serde_json::json!(client.provider_id())
                });
                if old.judgment.verdict == JudgeDecision::Accepted
                    && !defective.contains(&old.id)
                    && old.criterion == entry.criterion
                    && old.check == entry.check
                    && old.gap_permitted == entry.gap_permitted
                    && sampling_matches
                {
                    entry.judgment = old.judgment.clone();
                    reused.insert(entry.id.clone());
                }
            }
        }
    }
    let mut subset = contract.clone();
    subset
        .acceptance
        .retain(|entry| !reused.contains(&entry.id));
    subset
        .supplementary
        .retain(|entry| !reused.contains(&entry.id));
    let subset_ids: BTreeSet<_> = subset
        .acceptance
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    subset
        .gap_policy
        .permitted_acceptance_ids
        .retain(|id| subset_ids.contains(id));
    if !subset.acceptance.is_empty() || !subset.supplementary.is_empty() {
        let judged = judge_contract(client, subset, &subset_ids).await?;
        for entry in contract
            .acceptance
            .iter_mut()
            .chain(&mut contract.supplementary)
        {
            if let Some(new) = judged
                .acceptance
                .iter()
                .chain(&judged.supplementary)
                .find(|new| new.id == entry.id)
            {
                entry.judgment = new.judgment.clone();
            }
        }
    }
    // Preserve existing best-of behavior, but only from the verified base.
    if let Some(base) = &base {
        merge::keep_previously_accepted(&mut contract, base);
    }
    validate_acceptance_structure(&contract, expected, true).map_err(anyhow::Error::new)?;
    Ok(contract)
}
