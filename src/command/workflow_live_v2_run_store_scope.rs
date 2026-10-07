//! The run-store boundary for one live agent call.
//!
//! Built at the entry every agent call funnels through
//! (`run_single_v2_agent_call_in_repository`), so read-only single calls and
//! read-only fan-out branches get it as surely as write branches do. It used to
//! be set up only by the write dispatch, which left the read-only calls — the
//! ones most likely to walk the store — with no boundary and no walk pruning.

use archon_tools::workflow_read_guard::{AdmittedWrites, RunStoreScope};
use archon_workflow::WorkflowV2ProjectArtifactContext;
use archon_workflow::WorkflowV2ResultStore;

/// The scope for a call working in `working_root`, admitting what the call's
/// project-artifact context admits — the same rule the completion check reads
/// the call's reported deliverables by, so the two cannot disagree.
pub(crate) fn run_store_scope(
    v2_store: Option<&WorkflowV2ResultStore>,
    working_root: Option<&str>,
    artifacts: Option<&WorkflowV2ProjectArtifactContext>,
) -> RunStoreScope {
    let mut scope = RunStoreScope::new(
        v2_store
            .map(|store| store.run_root().display().to_string())
            .as_deref(),
        v2_store
            .and_then(|store| store.run_store_root())
            .map(|root| root.display().to_string())
            .as_deref(),
        working_root,
    );
    if let Some(v2) = v2_store.filter(|store| store.session_executor().is_some()) {
        // A malformed store is a refusal, never an unguarded dispatch.
        scope.admission = Some(match v2.session_workflow_store() {
            Ok(store) => store.admission_fence(&v2.run_id()),
            Err(error) => {
                let reason = error.to_string();
                archon_tools::workflow_read_guard::AdmissionFence::new(v2.run_id(), move || {
                    Err(archon_tools::workflow_read_guard::AdmissionStop {
                        kind: archon_tools::workflow_read_guard::StopKind::Refused,
                        reason: reason.clone(),
                    })
                })
            }
        });
    }
    match artifacts.filter(|context| !context.is_empty()) {
        Some(context) => {
            let context = context.clone();
            scope.with_admitted_writes(AdmittedWrites::new(move |path| {
                archon_workflow::project_artifact_write_admitted(&context, &path.to_string_lossy())
            }))
        }
        None => scope,
    }
}
