use super::*;

pub(super) async fn live_plan_from_generated_bundle(
    store: &WorkflowStore,
    run: &WorkflowRun,
) -> Result<Option<WorkflowScriptPlan>> {
    let manifest = match WorkflowBundle::verify(store, &run.id) {
        Ok(manifest) => manifest,
        Err(err) => {
            return Err(WorkflowError::SpecInvalid(format!(
                "workflow bundle verification failed for generated V2 run '{}': {err}",
                run.id
            ))
            .into());
        }
    };
    if !matches!(
        manifest.origin,
        WorkflowBundleOrigin::GeneratedHarness | WorkflowBundleOrigin::SavedCommand
    ) {
        return Ok(None);
    }
    let harness_path = archon_workflow::bundle::record_path(&store.run_dir(&run.id));
    let harness_source = fs::read_to_string(&harness_path).map_err(|err| WorkflowError::Io {
        path: harness_path.clone(),
        source: err,
    })?;
    // One source of truth for the plan: the host-call manifest persisted at
    // approval time. Scripts without one (older saved runs) are re-planned by
    // the QuickJS dry-run; a script that fails it is a hard error.
    let metadata = load_generated_v2_metadata(store, &run.id)?;
    let calls = match metadata
        .as_ref()
        .and_then(|metadata| metadata.generated_scaffold.as_ref())
        .map(|scaffold| scaffold.host_call_manifest.clone())
        .filter(|calls| !calls.is_empty())
    {
        Some(calls) => calls,
        None => dry_run_workflow_plan(&harness_source, None).await?,
    };
    let mut script_plan =
        WorkflowScriptPlan::from_template(run.spec.clone(), &harness_source, calls);
    if let Some(metadata) = metadata {
        let current_hash = workflow_scaffold_hash(&harness_source);
        if let Some(scaffold_hash) = metadata.scaffold_hash.as_deref()
            && scaffold_hash != current_hash
        {
            return Err(WorkflowError::ArtifactInvalid(format!(
                "generated V2 scaffold hash mismatch for run '{}': metadata {}, {} {}",
                run.id,
                scaffold_hash,
                harness_path.display(),
                current_hash
            ))
            .into());
        }
        if let Some(scaffold) = metadata.generated_scaffold.as_ref()
            && scaffold.scaffold_hash != current_hash
        {
            return Err(WorkflowError::ArtifactInvalid(format!(
                "generated V2 scaffold record hash mismatch for run '{}': metadata {}, {} {}",
                run.id,
                scaffold.scaffold_hash,
                harness_path.display(),
                current_hash
            ))
            .into());
        }
        script_plan.task_universe = metadata.task_universe;
        script_plan.script_args = metadata.script_args;
        script_plan.governed_learning_context = metadata.governed_learning_context;
        if let Some(generated_config) = metadata.generated_config {
            script_plan.generated_config = generated_config;
        }
    }
    Ok(Some(script_plan))
}
