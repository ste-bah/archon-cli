//! Runs and records the fixed read-only gates used during reverify.
use anyhow::Result;
use archon_workflow::{
    HostCommandRequest, HostCommandResult, RunStatus, WorkflowStore, WorkflowV2ResultStore,
};

use super::workflow_decompose_reverify::{GateResult, persist_call};
use crate::command::workflow_host_command_exec::{
    FixedHostCommandExecutor, WorkflowHostCommandExecutor,
};

pub(super) async fn execute_gate(
    executor: &FixedHostCommandExecutor,
    store: &WorkflowStore,
    v2: &WorkflowV2ResultStore,
    run_id: &str,
    run_dir: &std::path::Path,
    lease: &crate::command::workflow_executor_lease::ExecutionLease,
    generation: u64,
    status: &RunStatus,
    seq: u64,
    index: usize,
    command_id: &str,
    depends_on: &[String],
) -> Result<(HostCommandResult, GateResult)> {
    let subject = match command_id {
        "task-set-lint" | "requirements-trace" => "task-set",
        "verify-frozen-acceptance" | "verify-frozen-skeleton" => "frozen-task-set",
        _ => "workflow",
    };
    let request = HostCommandRequest::new(command_id, None)?;
    eprintln!("reverify gate started subject={subject} capability={command_id}");
    let result = executor
        .execute_observed_only(request.clone(), Some(generation))
        .await
        .unwrap_or_else(|error| {
            let gate_id = match command_id {
                "task-set-lint" => crate::command::workflow_gate::GateId::WorkflowLintTaskSet,
                "requirements-trace" => crate::command::workflow_gate::GateId::RequirementsTrace,
                _ => crate::command::workflow_gate::GateId::WorkflowLintTaskFile,
            };
            super::workflow_decompose_reverify_assessment::executor_error_result(
                error, gate_id, subject,
            )
        });
    let (gate_verdict, findings) =
        super::workflow_decompose_reverify_assessment::gate_assessment(&result);
    eprintln!(
        "reverify gate finished subject={subject} capability={command_id} verdict={gate_verdict}"
    );
    let call_id = format!("reverify-{seq}-gate-{index}-{command_id}");
    let logic_version = executor.logic_version(&request)?;
    let logic_digest = executor.logic_digest(&request)?;
    let build = executor.logic_build(&request)?;
    persist_call(
        executor, store, v2, run_id, run_dir, lease, generation, status, &call_id, request,
        &result, b"", depends_on,
    )?;
    let subjects = result
        .gate_envelope
        .as_ref()
        .map(|envelope| {
            envelope
                .policy_findings
                .iter()
                .map(|finding| finding.subject.clone())
                .collect()
        })
        .unwrap_or_default();
    let gate = GateResult {
        command_id: command_id.to_string(),
        call_id,
        logic_version,
        logic_digest,
        build,
        verdict: gate_verdict,
        findings,
        subjects,
    };
    Ok((result, gate))
}
