//! Native observer composition; all authority is re-derived from the live pin.
use std::{path::PathBuf,collections::BTreeSet};
use archon_workflow::{WorkflowStore,WorkflowError,WorkflowResult};
use archon_workflow::acceptance_scratch::{ScratchPolicy,ObservationResult};
use super::workflow_live_v2_finalizer::RunEndObserverContext;
use crate::command::acceptance_scratch_guardian::{Request,launch};

#[derive(Clone,serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeBinding { pub policy:ScratchPolicy,pub source_commit:String }

pub(super) async fn evaluate(store:&WorkflowStore,context:&RunEndObserverContext<'_>)->WorkflowResult<ObservationResult> {
    let binding:NativeBinding=serde_json::from_value(context.snapshot.native_execution.clone().ok_or_else(||WorkflowError::StateCorrupt("missing native observer policy".into()))?)?;
    let project=store.root().parent().and_then(std::path::Path::parent).ok_or_else(||WorkflowError::StateCorrupt("invalid project store".into()))?;
    let tasks=PathBuf::from(&context.snapshot.canonical_task_root_identity);
    if binding.policy.project.canonicalize().ok()!=project.canonicalize().ok() || binding.policy.task_root.canonicalize().ok()!=tasks.canonicalize().ok() {
        return Err(WorkflowError::StateCorrupt("native policy roots differ from observer snapshot".into()));
    }
    let pin_path=crate::command::workflow_task_set::acceptance_pin_path(project,&tasks);
    let bytes=std::fs::read(&pin_path).map_err(|e|WorkflowError::io(&pin_path,e))?;
    let pin:archon_workflow::task_set_contract::AcceptancePin=serde_json::from_slice(&bytes)?;
    let expected=context.snapshot.portable_acceptance_identity.as_ref().ok_or_else(||WorkflowError::StateCorrupt("native execution requires launch-bound pin identity".into()))?;
    if pin.acceptance_digest!=expected.acceptance_digest || pin.skeleton_digest!=expected.skeleton_digest || pin.freeze_event_id!=expected.freeze_event_id {
        return Err(WorkflowError::ArtifactInvalid("native observer chain differs from launch pin".into()));
    }
    let evidence=binding.policy.scratch_parent.join(format!("evidence-{}",uuid::Uuid::new_v4()));
    let result=launch(Request {policy:binding.policy,source_commit:binding.source_commit,pin_path,
        expected_pin_digest:archon_workflow::task_set_contract::content_digest(&bytes),evidence:evidence.clone()}).await?;
    store.write_run_json(context.run_id,"observer/native-observation.json",&result)?;
    // Evidence is retained; scratch worktrees/targets themselves were removed.
    Ok(result)
}
