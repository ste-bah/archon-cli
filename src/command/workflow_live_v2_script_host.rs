use super::*;

pub(super) struct WorkflowScriptHost {
    pub(super) scaffold_hash: String,
    pub(super) runner: WorkflowV2ScriptRunner,
    pub(super) accumulator: Arc<Mutex<WorkflowScriptAccumulator>>,
}

include!("workflow_live_v2_script_host_exec.rs");
include!("workflow_live_v2_script_host_state.rs");
include!("workflow_live_v2_script_host_events.rs");
