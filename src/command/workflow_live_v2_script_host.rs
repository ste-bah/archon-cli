use super::*;

pub(super) struct WorkflowScriptHost {
    pub(super) scaffold_hash: String,
    pub(super) runner: WorkflowV2ScriptRunner,
    pub(super) accumulator: Arc<Mutex<WorkflowScriptAccumulator>>,
}

#[path = "workflow_live_v2_script_host_exec.rs"]
mod workflow_live_v2_script_host_exec;
pub(crate) use workflow_live_v2_script_host_exec::*;
#[path = "workflow_live_v2_script_host_state.rs"]
mod workflow_live_v2_script_host_state;
pub(crate) use workflow_live_v2_script_host_state::*;
#[path = "workflow_live_v2_script_host_events.rs"]
mod workflow_live_v2_script_host_events;
pub(crate) use workflow_live_v2_script_host_events::*;
