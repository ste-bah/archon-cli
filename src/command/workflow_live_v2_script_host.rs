struct WorkflowScriptHost {
    scaffold_hash: String,
    runner: WorkflowV2ScriptRunner,
    accumulator: Arc<Mutex<WorkflowScriptAccumulator>>,
}

include!("workflow_live_v2_script_host_exec.rs");
include!("workflow_live_v2_script_host_state.rs");
include!("workflow_live_v2_script_host_events.rs");
