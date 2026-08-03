use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum GeneratedV2RestartTarget {
    Call(String),
    Item { call_id: String, item_id: String },
}

pub(super) fn generated_v2_restart_target(
    action: &LifecycleAction,
) -> Option<GeneratedV2RestartTarget> {
    match action {
        LifecycleAction::RestartStage(stage_id) => {
            Some(GeneratedV2RestartTarget::Call(stage_id.clone()))
        }
        LifecycleAction::RestartItem { stage_id, item_id } => {
            Some(GeneratedV2RestartTarget::Item {
                call_id: stage_id.clone(),
                item_id: item_id.clone(),
            })
        }
        _ => None,
    }
}

pub(super) fn invalidate_generated_v2_call(
    store: &WorkflowStore,
    run: &WorkflowRun,
    call_id: &str,
) -> Result<Vec<String>> {
    invalidate_generated_v2_call_cache(store, run, call_id, true)
}

pub(super) fn restart_generated_v2_task_workflow(
    store: &WorkflowStore,
    run: &WorkflowRun,
    task_id: &str,
) -> Result<Option<String>> {
    let _manifest = match WorkflowBundle::verify(store, &run.id) {
        Ok(manifest)
            if matches!(
                manifest.origin,
                WorkflowBundleOrigin::GeneratedHarness | WorkflowBundleOrigin::SavedCommand
            ) =>
        {
            manifest
        }
        _ => return Ok(None),
    };
    let Some(task_universe) = generated_v2_task_universe(store, run)? else {
        return Ok(None);
    };
    let canonical_task_id = task_universe.resolve_canonical_task_id(task_id)?;
    let affected_task_ids = task_universe.downstream_task_closure(&canonical_task_id);
    let executions = generated_v2_restart_executions(store, run)?;
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let invalidation = v2_store.invalidate_task_and_dependents(
        &executions,
        &canonical_task_id,
        &affected_task_ids,
        &format!("restart-task:{canonical_task_id}"),
    )?;
    reset_generated_v2_task_state(store, &run.id, &invalidation)?;
    Ok(Some(format_generated_v2_task_restart(
        &run.id,
        task_id,
        &invalidation,
    )))
}

fn generated_v2_task_universe(
    store: &WorkflowStore,
    run: &WorkflowRun,
) -> Result<Option<archon_workflow::task_universe::WorkflowV2TaskUniverse>> {
    let metadata_path = store.run_dir(&run.id).join("v2/generated-metadata.json");
    if !metadata_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&metadata_path)?;
    let metadata: serde_json::Value = serde_json::from_str(&raw)?;
    metadata
        .get("task_universe")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|err| anyhow!("invalid persisted generated V2 task universe: {err}"))
}

fn format_generated_v2_task_restart(
    run_id: &str,
    requested_task_id: &str,
    invalidation: &WorkflowV2TaskInvalidation,
) -> String {
    format!(
        "Workflow generated V2 task restart prepared: task {requested_task_id} resolved to {}.\naffected_tasks: {}\ninvalidated_calls: {}\ndeleted_branch_outcomes: {}\nNext: /workflow continue {run_id}\n",
        invalidation.requested_task_id,
        invalidation.affected_task_ids.join(", "),
        invalidation.invalidated_call_ids.join(", "),
        invalidation.deleted_branch_outcomes.len(),
    )
}

fn reset_generated_v2_task_state(
    store: &WorkflowStore,
    run_id: &str,
    invalidation: &WorkflowV2TaskInvalidation,
) -> Result<()> {
    let mut run = store.load_state(run_id)?;
    for call_id in &invalidation.invalidated_call_ids {
        if run.stages.contains_key(call_id) {
            run.stages
                .insert(call_id.clone(), StageState::pending(call_id.clone()));
        }
    }
    run.status = RunStatus::Running;
    store.save_state(&run)?;
    Ok(())
}

fn invalidate_generated_v2_call_cache(
    store: &WorkflowStore,
    run: &WorkflowRun,
    call_id: &str,
    clear_branch_outcomes: bool,
) -> Result<Vec<String>> {
    let _manifest = match WorkflowBundle::verify(store, &run.id) {
        Ok(manifest)
            if matches!(
                manifest.origin,
                WorkflowBundleOrigin::GeneratedHarness | WorkflowBundleOrigin::SavedCommand
            ) =>
        {
            manifest
        }
        _ => return Ok(Vec::new()),
    };
    let executions = generated_v2_restart_executions(store, run)?;
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    let mut invalidated = v2_store
        .invalidate_call_and_dependents(&executions, call_id)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    invalidated.extend(v2_store.invalidate_dynamic_wave_dependents(call_id)?);
    if clear_branch_outcomes {
        let deleted = v2_store.delete_branch_outcomes_for_call(call_id)?;
        if deleted > 0 {
            invalidated.insert(format!("{call_id}:branches({deleted})"));
        }
    }
    let invalidated = invalidated.into_iter().collect::<Vec<_>>();
    reset_generated_v2_stage_state(store, &run.id, &invalidated)?;
    Ok(invalidated)
}

fn generated_v2_restart_executions(
    store: &WorkflowStore,
    run: &WorkflowRun,
) -> Result<Vec<WorkflowV2CallExecution>> {
    // One source of truth: the host-call manifest persisted with the run at
    // approval time. Runs without one fall back to per-call invalidation —
    // the result store's input hashing re-executes stale dependents anyway.
    let metadata_path = store.run_dir(&run.id).join("v2/generated-metadata.json");
    if !metadata_path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&metadata_path)?;
    let metadata: serde_json::Value = serde_json::from_str(&raw)?;
    let calls: Vec<archon_workflow::WorkflowV2HostCall> = metadata
        .get("generated_scaffold")
        .and_then(|scaffold| scaffold.get("host_call_manifest"))
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|err| anyhow!("invalid persisted host call manifest: {err}"))?
        .unwrap_or_default();
    Ok(calls
        .into_iter()
        .map(|call| {
            let depends_on = call
                .options
                .source
                .as_deref()
                .map(source_call_ids_for_restart)
                .unwrap_or_default();
            WorkflowV2CallExecution {
                call,
                input: serde_json::Value::Null,
                depends_on,
            }
        })
        .collect())
}

fn source_call_ids_for_restart(source: &str) -> Vec<String> {
    let trimmed = source.trim();
    let body = trimmed
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(trimmed);
    body.split(',')
        .map(|part| {
            part.trim()
                .split_once('.')
                .map(|(head, _)| head)
                .unwrap_or_else(|| part.trim())
                .trim_matches(|ch| ch == '"' || ch == '\'')
                .to_string()
        })
        .filter(|part| !part.is_empty())
        .collect()
}

fn reset_generated_v2_stage_state(
    store: &WorkflowStore,
    run_id: &str,
    invalidated: &[String],
) -> Result<()> {
    let mut run = store.load_state(run_id)?;
    let mut changed = false;
    for call_id in invalidated {
        if call_id.contains(':') {
            continue;
        }
        if run.stages.contains_key(call_id) {
            run.stages
                .insert(call_id.clone(), StageState::pending(call_id.clone()));
            changed = true;
        }
    }
    if changed {
        store.save_state(&run)?;
    }
    Ok(())
}

pub(super) fn invalidate_generated_v2_item(
    store: &WorkflowStore,
    run: &WorkflowRun,
    call_id: &str,
    item_id: &str,
) -> Result<Vec<String>> {
    let mut invalidated = invalidate_generated_v2_call_cache(store, run, call_id, false)?;
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    for candidate in v2_branch_item_candidates(call_id, item_id) {
        if v2_store.delete_branch_outcome(call_id, &candidate)? {
            invalidated.push(format!("{call_id}:{candidate}"));
        }
    }
    Ok(invalidated)
}

fn v2_branch_item_candidates(call_id: &str, item_id: &str) -> Vec<String> {
    let mut candidates = vec![item_id.to_string()];
    let prefixed = format!("{call_id}-{item_id}");
    if !item_id.starts_with(&format!("{call_id}-")) {
        candidates.push(prefixed);
    }
    candidates.sort();
    candidates.dedup();
    candidates
}
