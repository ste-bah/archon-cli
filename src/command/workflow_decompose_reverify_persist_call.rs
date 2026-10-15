//! Persist a read-only host-command observation from reverify.

use anyhow::{Result, anyhow};
use archon_workflow::{
    HostCommandRequest, HostCommandResult, WorkflowV2CallRecord, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2Result, WorkflowV2ResultStore,
    WorkflowV2Status, task_set_contract::content_digest,
};

use super::gate_assessment;
use crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor;
use crate::command::workflow_host_command_outcome_stamp;

pub fn persist_call(
    executor: &dyn WorkflowHostCommandExecutor,
    v2: &WorkflowV2ResultStore,
    run_id: &str,
    call_id: &str,
    request: HostCommandRequest,
    result: &HostCommandResult,
    stdin: &[u8],
    depends_on: &[String],
) -> Result<serde_json::Value> {
    let accepted = result.reusable();
    let call = WorkflowV2HostCall {
        id: call_id.to_string(),
        method: WorkflowV2HostMethod::HostCommand,
        write_mode: None,
        options: WorkflowV2HostOptions {
            host_command: Some(request),
            ..Default::default()
        },
    };
    let mut typed = WorkflowV2Result::default();
    typed.status = if accepted {
        WorkflowV2Status::Accepted
    } else {
        WorkflowV2Status::NeedsReview
    };
    typed.summary = if accepted {
        "reverify gate accepted".to_string()
    } else {
        let (verdict, findings) = gate_assessment(result);
        format!("reverify gate {verdict}: {}", findings.join("; "))
    };
    typed.data = workflow_host_command_outcome_stamp::stamp_host_outcome(
        Some(executor),
        &call,
        serde_json::to_value(result)?,
    )?;
    let stamped_data = typed.data.clone();
    let record = WorkflowV2CallRecord::new(
        run_id,
        call,
        1,
        content_digest(stdin),
        typed,
        depends_on.to_vec(),
    );
    if v2.result_path(call_id).exists() {
        return Err(anyhow!("reverify call record already exists: {call_id}"));
    }
    v2.save_call_record(&record)?;
    Ok(stamped_data)
}
