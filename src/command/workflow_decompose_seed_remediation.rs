use super::*;

/// A completed run normally has no phase to continue. A remediation ledger
/// reopens selected body phases, so derive their ordinal floors from the same
/// records and frozen-chain decision used by ordinary resume seeds. The
/// current task files are the inputs those reopened phases author from; do
/// not carry a prior candidate back into the body loop.
pub(crate) fn completed_remediation_seed(
    store: &WorkflowStore,
    run_id: &str,
    runtime: &FixedRunIdentityV1,
    criteria: &BTreeMap<String, String>,
    requirement_texts: &BTreeMap<String, String>,
    resume_frozen_chain: &serde_json::Value,
    ledger: Option<&crate::command::workflow_decompose_remediation::Ledger>,
) -> Result<Option<PhaseSeed>> {
    if ledger.is_none()
        || !matches!(
            store.load_state(run_id)?.status,
            archon_workflow::RunStatus::Completed | archon_workflow::RunStatus::Paused
        )
    {
        return Ok(None);
    }
    let generation = store.load_state(run_id)?.generation;
    let mut seed = derive_seed(
        store,
        run_id,
        (generation as usize, runtime.clone()),
        criteria,
        requirement_texts,
        Some(resume_frozen_chain),
        &[],
    )?;
    seed.subjects.clear();
    Ok(Some(seed))
}
