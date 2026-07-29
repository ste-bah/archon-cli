use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use archon_tui::app::TuiEvent;
use archon_workflow::{
    WorkflowError, WorkflowEventKind, WorkflowEventLog, WorkflowStore, WorkflowV2AgentAdapter,
    WorkflowV2ArtifactRequirement, WorkflowV2CallExecution, WorkflowV2CallRecord,
    WorkflowV2Checkpoint, WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostCall,
    WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2ResidualGap, WorkflowV2Result,
    WorkflowV2ResultStore, WorkflowV2Status, WorkflowV2TaskCompletionEvidence,
    WorkflowV2TaskCoverageStatus, WorkflowV2WriteMode, workflow_scaffold_hash,
};
use rquickjs::function::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
use tokio::sync::Mutex;

use super::super::workflow_live_task_universe::WorkflowV2TaskUniverse;
use super::WorkflowV2ScriptRuntime;
use super::execute_v2_live_call;
use super::workflow_live_v2_client::LiveV2AgentClient;
use super::workflow_live_v2_contracts::failed_v2_result;
use super::workflow_live_v2_source_graph::{
    complete_source_task_graph, dynamic_wave_source_metadata, input_hash_with_source_fingerprint,
};
use super::workflow_live_v2_stable_json::stable_hash;
use super::workflow_live_v2_state::{mark_v2_call_running, poll_v2_run_control};

const TERMINAL_HOST_CALL_MARKER: &str = "workflow terminal host call:";
#[cfg(not(test))]
const WORKFLOW_JS_WATCHDOG: Duration = Duration::from_secs(60);
#[cfg(test)]
const WORKFLOW_JS_WATCHDOG: Duration = Duration::from_millis(250);

#[derive(Debug, Clone)]
pub(super) struct WorkflowV2ScriptSummary {
    pub(super) status: WorkflowV2Status,
    pub(super) completed: usize,
    pub(super) executed: usize,
    pub(super) reused: usize,
    pub(super) calls: Vec<WorkflowV2HostCall>,
    pub(super) failed_call: Option<String>,
    pub(super) failed_result_path: Option<String>,
    pub(super) next_action: Option<String>,
}

#[derive(Clone)]
pub(super) struct WorkflowV2ScriptRunner {
    task: String,
    runtime: WorkflowV2ScriptRuntime,
    adapter: WorkflowV2AgentAdapter,
    client: LiveV2AgentClient,
    v2_store: WorkflowV2ResultStore,
    workflow_store: WorkflowStore,
    run_id: String,
    workspace_boundary_supported: bool,
    task_universe: Option<WorkflowV2TaskUniverse>,
    script_args: Option<serde_json::Value>,
    adopt_accepted_cache: bool,
    resume_completed_ids: std::collections::BTreeSet<String>,
}

impl WorkflowV2ScriptRunner {
    pub(super) fn new(
        task: String,
        runtime: WorkflowV2ScriptRuntime,
        adapter: WorkflowV2AgentAdapter,
        client: LiveV2AgentClient,
        v2_store: WorkflowV2ResultStore,
        workflow_store: WorkflowStore,
        run_id: String,
        workspace_boundary_supported: bool,
        task_universe: Option<WorkflowV2TaskUniverse>,
        script_args: Option<serde_json::Value>,
    ) -> Self {
        Self {
            task,
            runtime,
            adapter,
            client,
            v2_store,
            workflow_store,
            run_id,
            workspace_boundary_supported,
            task_universe,
            script_args,
            adopt_accepted_cache: false,
            resume_completed_ids: Default::default(),
        }
    }

    pub(super) fn with_frontier_resume(mut self, enabled: bool) -> Self {
        self.adopt_accepted_cache = enabled;
        self
    }

    pub(super) fn with_resume_completed_ids(
        mut self,
        completed_ids: std::collections::BTreeSet<String>,
    ) -> Self {
        self.resume_completed_ids = completed_ids;
        self
    }

    pub(super) async fn run(
        self,
        harness_source: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let harness_source = harness_source.to_string();
        tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| {
                    WorkflowError::SpecInvalid(format!(
                        "workflow.js local async runtime failed: {err}"
                    ))
                })?;
            runtime.block_on(self.run_on_current_thread(&harness_source))
        })
        .await
        .map_err(|err| WorkflowError::SpecInvalid(format!("workflow.js task failed: {err}")))?
    }

    async fn run_on_current_thread(
        self,
        harness_source: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let script_args = self.script_args.clone();
        let host = Arc::new(WorkflowScriptHost {
            scaffold_hash: workflow_scaffold_hash(harness_source),
            runner: self,
            accumulator: Arc::new(Mutex::new(WorkflowScriptAccumulator::default())),
        });
        let runtime = AsyncRuntime::new()
            .map_err(|err| WorkflowError::SpecInvalid(format!("quickjs runtime failed: {err}")))?;
        let watchdog = WorkflowJsWatchdog::new();
        let watchdog_for_interrupt = watchdog.clone();
        runtime
            .set_interrupt_handler(Some(Box::new(move || {
                watchdog_for_interrupt.should_interrupt()
            })))
            .await;
        let context = AsyncContext::full(&runtime)
            .await
            .map_err(|err| WorkflowError::SpecInvalid(format!("quickjs context failed: {err}")))?;
        let source = script_source(harness_source, script_args.as_ref());
        let host_for_js = host.clone();
        let watchdog_for_js = watchdog.clone();
        let js_result = context
            .async_with(async move |ctx| {
                ctx.globals().set(
                    "__archonHost",
                    Func::from(Async(move |method: String, payload: String| {
                        let host = host_for_js.clone();
                        let watchdog = watchdog_for_js.clone();
                        async move {
                            watchdog.pause();
                            let result = host.execute(method, payload).await;
                            watchdog.resume();
                            result.map_err(|err| {
                                rquickjs::Error::new_from_js_message(
                                    "archon workflow host",
                                    "string",
                                    err.to_string(),
                                )
                            })
                        }
                    })),
                )?;
                let promise: Promise = match ctx.eval(source.as_str()).catch(&ctx) {
                    Ok(promise) => promise,
                    Err(err) => {
                        return Err(rquickjs::Error::new_from_js_message(
                            "workflow.js",
                            "promise",
                            err.to_string(),
                        ));
                    }
                };
                match promise.into_future::<String>().await.catch(&ctx) {
                    Ok(result) => Ok(result),
                    Err(err) => Err(rquickjs::Error::new_from_js_message(
                        "workflow.js",
                        "string",
                        err.to_string(),
                    )),
                }
            })
            .await;
        match js_result {
            Ok(_) => {
                let summary = host.summary().await;
                host.emit_terminal_status(summary.status);
                Ok(summary)
            }
            Err(err) => {
                let error = err.to_string();
                if error.contains(TERMINAL_HOST_CALL_MARKER) {
                    let summary = host.summary().await;
                    host.emit_terminal_status(summary.status);
                    return Ok(summary);
                }
                let workflow_error = workflow_js_error(error.clone());
                if matches!(
                    workflow_error,
                    WorkflowError::ControlPaused(_)
                        | WorkflowError::ControlCancelled(_)
                        | WorkflowError::NotificationDelivery(_)
                ) {
                    return Err(workflow_error);
                }
                let summary = host.mark_script_failure(&error).await;
                Ok(summary)
            }
        }
    }
}

#[derive(Clone)]
struct WorkflowJsWatchdog {
    active_since: Arc<StdMutex<Option<Instant>>>,
}

impl WorkflowJsWatchdog {
    fn new() -> Self {
        Self {
            active_since: Arc::new(StdMutex::new(Some(Instant::now()))),
        }
    }

    fn pause(&self) {
        if let Ok(mut active_since) = self.active_since.lock() {
            *active_since = None;
        }
    }

    fn resume(&self) {
        if let Ok(mut active_since) = self.active_since.lock() {
            *active_since = Some(Instant::now());
        }
    }

    fn should_interrupt(&self) -> bool {
        let Ok(active_since) = self.active_since.lock() else {
            return true;
        };
        active_since.is_some_and(|started| started.elapsed() >= WORKFLOW_JS_WATCHDOG)
    }
}

fn workflow_js_error(error: String) -> WorkflowError {
    if let Some(message) = extract_run_control_message(&error, "workflow paused by run control:") {
        return WorkflowError::ControlPaused(message);
    }
    if let Some(message) = extract_run_control_message(&error, "workflow cancelled by run control:")
    {
        return WorkflowError::ControlCancelled(message);
    }
    if let Some(message) =
        extract_run_control_message(&error, "required workflow notification delivery failed:")
    {
        return WorkflowError::NotificationDelivery(message);
    }
    WorkflowError::SpecInvalid(format!("workflow.js execution failed: {error}"))
}

fn extract_run_control_message(error: &str, marker: &str) -> Option<String> {
    let start = error.find(marker)?;
    Some(error[start..].trim().to_string())
}

struct WorkflowScriptAccumulator {
    status: WorkflowV2Status,
    completed: usize,
    executed: usize,
    reused: usize,
    calls: Vec<WorkflowV2HostCall>,
    failed_call: Option<String>,
    failed_result_path: Option<String>,
    next_action: Option<String>,
}

impl Default for WorkflowScriptAccumulator {
    fn default() -> Self {
        Self {
            status: WorkflowV2Status::Accepted,
            completed: 0,
            executed: 0,
            reused: 0,
            calls: Vec::new(),
            failed_call: None,
            failed_result_path: None,
            next_action: None,
        }
    }
}

include!("workflow_live_v2_script_host.rs");

include!("workflow_live_v2_script_helpers.rs");

include!("workflow_live_v2_script_verification.rs");

#[path = "workflow_live_v2_deliverable_contract.rs"]
mod workflow_live_v2_deliverable_contract;
#[path = "workflow_live_v2_lifecycle_noop_routing.rs"]
mod workflow_live_v2_lifecycle_noop_routing;
#[path = "workflow_live_v2_lifecycle_prompts.rs"]
mod workflow_live_v2_lifecycle_prompts;
#[path = "workflow_live_v2_lifecycle_review_remediation.rs"]
mod workflow_live_v2_lifecycle_review_remediation;
#[path = "workflow_live_v2_lifecycle_review_verification.rs"]
mod workflow_live_v2_lifecycle_review_verification;
#[path = "workflow_live_v2_lifecycle_terminal_gate.rs"]
mod workflow_live_v2_lifecycle_terminal_gate;
#[path = "workflow_live_v2_lifecycle_verify_invariants.rs"]
mod workflow_live_v2_lifecycle_verify_invariants;
#[path = "workflow_live_v2_lifecycle_verify_merge.rs"]
mod workflow_live_v2_lifecycle_verify_merge;
#[path = "workflow_live_v2_lifecycle_verify_options.rs"]
mod workflow_live_v2_lifecycle_verify_options;
#[path = "workflow_live_v2_lifecycle_verify_outcome_repair.rs"]
mod workflow_live_v2_lifecycle_verify_outcome_repair;
#[path = "workflow_live_v2_lifecycle_verify_overreach.rs"]
mod workflow_live_v2_lifecycle_verify_overreach;
#[path = "workflow_live_v2_lifecycle_verify_retriage.rs"]
mod workflow_live_v2_lifecycle_verify_retriage;
#[path = "workflow_live_v2_lifecycle_verify_routing.rs"]
mod workflow_live_v2_lifecycle_verify_routing;
#[path = "workflow_live_v2_lifecycle_verify_scope.rs"]
mod workflow_live_v2_lifecycle_verify_scope;
#[path = "workflow_live_v2_lifecycle_verify_supersede.rs"]
mod workflow_live_v2_lifecycle_verify_supersede;

include!("workflow_live_v2_script_dry_run.rs");

include!("workflow_live_v2_lifecycle.rs");

include!("workflow_live_v2_lifecycle_waves.rs");

include!("workflow_live_v2_lifecycle_impl.rs");

include!("workflow_live_v2_lifecycle_verify.rs");

include!("workflow_live_v2_lifecycle_verify_triage.rs");

include!("workflow_live_v2_lifecycle_verify_remediation.rs");

include!("workflow_live_v2_lifecycle_verify_outcome_repair_driver.rs");

include!("workflow_live_v2_lifecycle_review.rs");

#[cfg(test)]
#[path = "workflow_live_v2_script_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_e2e_tests.rs"]
mod workflow_live_v2_lifecycle_e2e_tests;
#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_review_remediation_tests.rs"]
mod workflow_live_v2_lifecycle_review_remediation_tests;
#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_review_verification_tests.rs"]
mod workflow_live_v2_lifecycle_review_verification_tests;
#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_verify_options_tests.rs"]
mod workflow_live_v2_lifecycle_verify_options_tests;
#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_verify_outcome_repair_tests.rs"]
mod workflow_live_v2_lifecycle_verify_outcome_repair_tests;
#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_verify_remediation_tests.rs"]
mod workflow_live_v2_lifecycle_verify_remediation_tests;
