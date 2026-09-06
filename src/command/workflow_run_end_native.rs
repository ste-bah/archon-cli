//! Native observer composition; all authority is re-derived from the live pin.
use super::workflow_live_v2_finalizer::RunEndObserverContext;
use crate::command::acceptance_scratch_guardian::{Request, launch};
use archon_workflow::acceptance_scratch::ObservationResult;
use archon_workflow::{WorkflowError, WorkflowResult, WorkflowStore};
use std::path::PathBuf;

use crate::command::acceptance_scratch_policy::NativeBinding;

pub(super) async fn evaluate(
    store: &WorkflowStore,
    context: &RunEndObserverContext<'_>,
) -> WorkflowResult<ObservationResult> {
    let result = evaluate_inner(store, context).await;
    if let Err(error) = &result {
        let path = store
            .run_dir(context.run_id)
            .join("observer/native-observation.json");
        if !path.exists() {
            store.write_run_json(
                context.run_id,
                "observer/native-observation.json",
                &serde_json::json!({
                    "operational_errors":[error.to_string()], "teardown_verified":false,
                    "live_roots_unchanged":false, "checks":[]
                }),
            )?;
        }
    }
    result
}
async fn evaluate_inner(
    store: &WorkflowStore,
    context: &RunEndObserverContext<'_>,
) -> WorkflowResult<ObservationResult> {
    let terminal_path = store.run_dir(context.run_id).join("v2/finalization.json");
    let terminal: archon_workflow::FinalizationRecordV1 =
        serde_json::from_slice(&std::fs::read(&terminal_path).map_err(|_| {
            WorkflowError::StateCorrupt(
                "native observation requires persisted terminal state and event".into(),
            )
        })?)?;
    let run = store.load_state(context.run_id)?;
    if !terminal.terminal_state_committed
        || !terminal.terminal_event_committed
        || terminal.terminal_v2_status != Some(context.terminal_status)
        || run.status != terminal.terminal_status
        || !matches!(
            run.status,
            archon_workflow::RunStatus::Completed | archon_workflow::RunStatus::NeedsReview
        )
        || terminal.observer_snapshot.as_ref() != Some(context.snapshot)
    {
        return Err(WorkflowError::StateCorrupt(
            "native observation terminal identity or persistence differs".into(),
        ));
    }
    let binding: NativeBinding =
        serde_json::from_value(context.snapshot.native_execution.clone().ok_or_else(|| {
            WorkflowError::StateCorrupt("missing native observer policy".into())
        })?)?;
    let project = super::workflow_run_end_snapshot::project_root(store)
        .ok_or_else(|| WorkflowError::StateCorrupt("invalid project store".into()))?;
    let tasks = PathBuf::from(&context.snapshot.canonical_task_root_identity);
    if binding.policy.project.canonicalize().ok() != project.canonicalize().ok()
        || binding.policy.task_root.canonicalize().ok() != tasks.canonicalize().ok()
    {
        return Err(WorkflowError::StateCorrupt(
            "native policy roots differ from observer snapshot".into(),
        ));
    }
    let pin_path = crate::command::workflow_task_set::acceptance_pin_path(project, &tasks);
    let bytes = std::fs::read(&pin_path).map_err(|e| WorkflowError::Io {
        path: pin_path.clone(),
        source: e,
    })?;
    let pin: archon_workflow::task_set_contract::AcceptancePin = serde_json::from_slice(&bytes)?;
    let expected = context
        .snapshot
        .portable_acceptance_identity
        .as_ref()
        .ok_or_else(|| {
            WorkflowError::StateCorrupt(
                "native execution requires launch-bound pin identity".into(),
            )
        })?;
    if pin.acceptance_digest != expected.acceptance_digest
        || pin.skeleton_digest != expected.skeleton_digest
        || pin.freeze_event_id != expected.freeze_event_id
    {
        return Err(WorkflowError::ArtifactInvalid(
            "native observer chain differs from launch pin".into(),
        ));
    }
    let evidence = binding
        .policy
        .scratch_parent
        .join(format!("evidence-{}", uuid::Uuid::new_v4()));
    let result = launch(Request {
        policy: binding.policy,
        source_commit: binding.source_commit,
        pin_path,
        expected_pin_digest: archon_workflow::task_set_contract::content_digest(&bytes),
        evidence: evidence.clone(),
    })
    .await;
    if let Err(error) = &result {
        let raw = std::fs::read(evidence.join("observation.json"))
            .ok().and_then(|bytes|serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .unwrap_or_else(||serde_json::json!({"operational_errors":[error.to_string()],"teardown_verified":false}));
        store.write_run_json(context.run_id, "observer/native-observation.json", &raw)?;
    }
    let result = result?;
    store.write_run_json(context.run_id, "observer/native-observation.json", &result)?;
    // Evidence is retained; scratch worktrees/targets themselves were removed.
    Ok(result)
}
