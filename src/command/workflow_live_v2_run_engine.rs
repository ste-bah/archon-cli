//! A resumed executor keeps the engine that authored its persisted artifacts.
use super::*;

pub(super) fn stored_engine_choice(store: &WorkflowStore, run_id: &str) -> bool {
    // Decomposed-PRD runs default to the Rust lifecycle. v3 script mode (ARCHON_SCRIPT_LIFECYCLE=1)
    // instead AUTHORS a workflow.js from the task universe and executes it.
    // A CONTINUED run MUST use the lifecycle it was created with (persisted in
    // metadata): re-reading the env var here silently switches a v3 run to
    // decomposed when the flag is absent, and the decomposed engine cannot
    // reuse the v3 run's records — it re-does everything under a different
    // engine. Persisted choice wins; the env var is only the fallback for a
    // run that predates this field.
    load_generated_v2_metadata(store, run_id)
        .ok()
        .flatten()
        .and_then(|metadata| metadata.script_lifecycle)
        // Legacy runs created before the persisted field: a v3 run leaves an
        // authored-workflow.js in its run dir — detect it so those continue as
        // v3 too, rather than falling back to the env var and switching engine.
        .or_else(|| {
            store
                .run_dir(run_id)
                .join("authored-workflow.js")
                .exists()
                .then_some(true)
        })
        .unwrap_or_else(script_lifecycle_from_env)
}
