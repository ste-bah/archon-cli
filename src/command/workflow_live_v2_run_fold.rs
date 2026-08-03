//! Everything a finished run projects outwards.
//!
//! Split out of `workflow_live_v2_run.rs` when the SONA outcome recording
//! pushed that file past the 500-line limit. It is a natural seam: nothing here
//! runs while the workflow does, and nothing here may change what the run
//! reports.

use std::path::Path;

use archon_workflow::WorkflowStore;

/// Project a finished workflow run into the topology corpus and the learning
/// stack.
///
/// Graph completion is the trigger the design names, and this is it for
/// `/workflow`: the run's `events.jsonl` becomes a topology trace and a single
/// batched fold writes `.archon/topology.db` plus one `learning_events`
/// summary row; then the learning bridge writes the run's record stream and
/// routes it by the spec's `learning_hooks` into `LearningIntegration`; then
/// the parameter tuner records what the run's generated limits were under.
///
/// Runs on `spawn_blocking` because every part is synchronous and the Cozo
/// write guard's retry loop sleeps on `thread::sleep` — roughly 19 seconds
/// worst case, which on a tokio worker is a runtime stall.
///
/// Entirely best-effort: a failure to record must never change what the user's
/// run reports.
pub(super) async fn fold_run_topology(
    cwd: &Path,
    store: &WorkflowStore,
    run_id: &str,
    task: &str,
    learning: &archon_core::config::LearningConfig,
) {
    let cwd = cwd.to_path_buf();
    let store = store.clone();
    let run_id = run_id.to_string();
    let task = task.to_string();
    let learning = learning.clone();
    let _ = tokio::task::spawn_blocking(move || {
        crate::command::topology_trace::project_workflow_run(&cwd, &store, &run_id);
        crate::command::topology_fold::fold_project_pending_blocking(
            &cwd, &run_id, &task, "default",
        );
        crate::command::topology_fold::bridge_workflow_learning(&cwd, &store, &run_id);
        // The write half of the parameter loop, and the reason SONA is no
        // longer recording-only. It runs here rather than beside the learning
        // bridge because it reads the run's persisted call records, which only
        // exist once the run is finished, and it is keyed by the same
        // `classify_generated_run` the read side used — a run whose outcome
        // were filed under a different class than the weight it consumed would
        // never converge on anything.
        let class =
            crate::command::workflow_live_learning_hooks::classify_generated_run(&task, None);
        crate::command::workflow_live_sona_tuning::record_generated_tuning_outcome(
            &cwd,
            &store,
            &run_id,
            class.as_str(),
            &learning,
        );
    })
    .await;
}
