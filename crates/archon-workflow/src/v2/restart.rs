//! Restarting part of a generated V2 run.
//!
//! A restart is not just a lifecycle transition. The V2 result store caches
//! every host call by input hash, so re-running a call that already has a
//! cached result replays the cached result instead of executing. Restarting a
//! call therefore means invalidating that call's cache entry AND every entry
//! downstream of it, then putting the corresponding stage states back to
//! pending so the run has somewhere to resume from.
//!
//! This lives beside the result store rather than in the CLI because every
//! path here is knowledge of the run directory's layout — where the host-call
//! manifest is persisted, how a branch outcome is keyed, which stage states a
//! cache invalidation implies. The CLI's part is choosing a restart target and
//! rendering the outcome; the outcome itself is a
//! [`WorkflowV2TaskInvalidation`], not a sentence.

use std::collections::BTreeSet;
use std::fs;

use crate::bundle::{WorkflowBundle, WorkflowBundleOrigin};
use crate::error::{WorkflowError, WorkflowResult};
use crate::lifecycle::LifecycleAction;
use crate::run::{RunStatus, StageState, WorkflowRun};
use crate::store::WorkflowStore;
use crate::task_universe::WorkflowV2TaskUniverse;

use super::call_execution::WorkflowV2CallExecution;
use super::host_api::WorkflowV2HostCall;
use super::result_store::{WorkflowV2ResultStore, WorkflowV2TaskInvalidation};

/// Where the run directory keeps the generated run's own description of itself.
///
/// `crate::approval` names the same path for the approval subject; both read
/// the file the live host writes at plan time.
const GENERATED_METADATA_FILE: &str = "v2/generated-metadata.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeneratedV2RestartTarget {
    Call(String),
    Item { call_id: String, item_id: String },
}

pub fn generated_v2_restart_target(action: &LifecycleAction) -> Option<GeneratedV2RestartTarget> {
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

pub fn invalidate_generated_v2_call(
    store: &WorkflowStore,
    run: &WorkflowRun,
    call_id: &str,
) -> WorkflowResult<Vec<String>> {
    invalidate_generated_v2_call_cache(store, run, call_id, true)
}

/// Invalidate a task and everything downstream of it.
///
/// `Ok(None)` means this run is not a generated V2 run, or carries no task
/// universe — there is nothing task-shaped to restart and the caller falls back
/// to a stage restart.
pub fn restart_generated_v2_task(
    store: &WorkflowStore,
    run: &WorkflowRun,
    task_id: &str,
) -> WorkflowResult<Option<WorkflowV2TaskInvalidation>> {
    if !is_generated_v2_bundle(store, run) {
        return Ok(None);
    }
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
    Ok(Some(invalidation))
}

pub fn invalidate_generated_v2_item(
    store: &WorkflowStore,
    run: &WorkflowRun,
    call_id: &str,
    item_id: &str,
) -> WorkflowResult<Vec<String>> {
    let mut invalidated = invalidate_generated_v2_call_cache(store, run, call_id, false)?;
    let v2_store = WorkflowV2ResultStore::new(store.run_dir(&run.id).join("v2"));
    for candidate in v2_branch_item_candidates(call_id, item_id) {
        if v2_store.delete_branch_outcome(call_id, &candidate)? {
            invalidated.push(format!("{call_id}:{candidate}"));
        }
    }
    Ok(invalidated)
}

fn is_generated_v2_bundle(store: &WorkflowStore, run: &WorkflowRun) -> bool {
    matches!(
        WorkflowBundle::verify(store, &run.id),
        Ok(manifest)
            if matches!(
                manifest.origin,
                WorkflowBundleOrigin::GeneratedHarness | WorkflowBundleOrigin::SavedCommand
            )
    )
}

fn generated_v2_task_universe(
    store: &WorkflowStore,
    run: &WorkflowRun,
) -> WorkflowResult<Option<WorkflowV2TaskUniverse>> {
    let metadata_path = store.run_dir(&run.id).join(GENERATED_METADATA_FILE);
    if !metadata_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&metadata_path).map_err(|err| WorkflowError::Io {
        path: metadata_path.clone(),
        source: err,
    })?;
    let metadata: serde_json::Value = serde_json::from_str(&raw)?;
    metadata
        .get("task_universe")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|err| {
            WorkflowError::StateCorrupt(format!(
                "invalid persisted generated V2 task universe: {err}"
            ))
        })
}

fn reset_generated_v2_task_state(
    store: &WorkflowStore,
    run_id: &str,
    invalidation: &WorkflowV2TaskInvalidation,
) -> WorkflowResult<()> {
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
) -> WorkflowResult<Vec<String>> {
    if !is_generated_v2_bundle(store, run) {
        return Ok(Vec::new());
    }
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
) -> WorkflowResult<Vec<WorkflowV2CallExecution>> {
    // One source of truth: the host-call manifest persisted with the run at
    // approval time. Runs without one fall back to per-call invalidation —
    // the result store's input hashing re-executes stale dependents anyway.
    let metadata_path = store.run_dir(&run.id).join(GENERATED_METADATA_FILE);
    if !metadata_path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&metadata_path).map_err(|err| WorkflowError::Io {
        path: metadata_path.clone(),
        source: err,
    })?;
    let metadata: serde_json::Value = serde_json::from_str(&raw)?;
    let calls: Vec<WorkflowV2HostCall> = metadata
        .get("generated_scaffold")
        .and_then(|scaffold| scaffold.get("host_call_manifest"))
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|err| {
            WorkflowError::StateCorrupt(format!("invalid persisted host call manifest: {err}"))
        })?
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
) -> WorkflowResult<()> {
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
