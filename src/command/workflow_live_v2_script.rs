use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use archon_workflow::{
    WorkflowError, WorkflowEventKind, WorkflowEventLog, WorkflowStore, WorkflowUiEvent,
    WorkflowV2AgentAdapter, WorkflowV2ArtifactRequirement, WorkflowV2CallExecution,
    WorkflowV2CallRecord, WorkflowV2Checkpoint, WorkflowV2Evidence, WorkflowV2EvidenceKind,
    WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2HostOptions, WorkflowV2ResidualGap,
    WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status, WorkflowV2TaskCompletionEvidence,
    WorkflowV2TaskCoverageStatus, WorkflowV2WriteMode, workflow_scaffold_hash,
};
use rquickjs::function::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
use tokio::sync::Mutex;

use super::WorkflowV2ScriptRuntime;
use super::execute_v2_live_call;
use super::workflow_live_v2_client::LiveV2AgentClient;
use super::workflow_live_v2_contracts::failed_v2_result;
use super::workflow_live_v2_source_graph::{
    complete_source_task_graph, dynamic_wave_source_metadata, input_hash_with_source_fingerprint,
};
use super::workflow_live_v2_stable_json::stable_hash;
use super::workflow_live_v2_state::{mark_v2_call_running, poll_v2_run_control};
use archon_workflow::task_universe::WorkflowV2TaskUniverse;

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
    /// The script's own return value (JSON text). Consumed by the v3
    /// authoring bootstrap to hand back the authored workflow source.
    pub(super) script_result: Option<String>,
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
    /// Canonical task ids whose work RE-EXECUTED during THIS run, closed over
    /// the task universe's dependency edges.
    ///
    /// The store's invalidation routines only ever fire from the operator's
    /// `workflow restart` command; nothing marks a downstream record stale when
    /// an upstream call re-executes mid-run and produces different output. The
    /// content-keyed reuse paths do not need that — a changed input changes the
    /// input hash and they re-execute on their own. The two reuse paths that
    /// legitimately cannot key on the input hash do, so they consult this set
    /// instead: reuse is refused for any record covering a task that is
    /// downstream of work this run has already redone.
    ///
    /// Shared by `Arc` across runner clones on purpose: the v3 authoring
    /// bootstrap and the authored run it hands off to are one logical run, and
    /// taint must not be laundered by the clone.
    reexecuted_task_closure: Arc<StdMutex<std::collections::BTreeSet<String>>>,
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
            reexecuted_task_closure: Arc::new(StdMutex::new(Default::default())),
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
        self.run_with_terminal_status(harness_source, true).await
    }

    pub(super) async fn run_without_terminal_status(
        self,
        harness_source: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        self.run_with_terminal_status(harness_source, false).await
    }

    async fn run_with_terminal_status(
        self,
        harness_source: &str,
        emit_terminal_status: bool,
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
            runtime.block_on(self.run_on_current_thread(&harness_source, emit_terminal_status))
        })
        .await
        .map_err(|err| WorkflowError::SpecInvalid(format!("workflow.js task failed: {err}")))?
    }

    async fn run_on_current_thread(
        self,
        harness_source: &str,
        emit_terminal_status: bool,
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
            Ok(result) => {
                let mut summary = host.summary().await;
                summary.script_result = Some(result);
                if emit_terminal_status {
                    host.emit_terminal_status(summary.status);
                }
                Ok(summary)
            }
            Err(err) => {
                let error = err.to_string();
                if error.contains(TERMINAL_HOST_CALL_MARKER) {
                    let summary = host.summary().await;
                    if emit_terminal_status {
                        host.emit_terminal_status(summary.status);
                    }
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
                let summary = host.mark_script_failure(&error, emit_terminal_status).await;
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

#[path = "workflow_live_v2_script_host.rs"]
mod workflow_live_v2_script_host;
use workflow_live_v2_script_host::*;

#[path = "workflow_live_v2_script_helpers.rs"]
mod workflow_live_v2_script_helpers;
use workflow_live_v2_script_helpers::*;
// Re-exported: `workflow_live_v2` names this through the subsystem, which the
// splice used to satisfy implicitly.
pub(super) use workflow_live_v2_script_helpers::frontier_resume_record_reusable;

#[path = "workflow_live_v3_prelude.rs"]
mod workflow_live_v3_prelude;
use workflow_live_v3_prelude::*;

#[path = "workflow_live_v2_script_verification.rs"]
mod workflow_live_v2_script_verification;
use workflow_live_v2_script_verification::*;

// The lifecycle's value-level policy now lives in archon-workflow. One import
// here, and the driver parts below reach every module through it.
use archon_workflow::v2::lifecycle_policy;

#[path = "workflow_live_v2_lifecycle_review_remediation.rs"]
mod workflow_live_v2_lifecycle_review_remediation;
#[path = "workflow_live_v2_lifecycle_review_verification.rs"]
mod workflow_live_v2_lifecycle_review_verification;
// Whole-pipeline plan generation over the real 17-task PRD fixture. It lives
// inside this subsystem because that is the only scope from which the planner,
// the task universe, the scheduler primitives and the per-task review item
// builder are all reachable at once — which is exactly the property that made
// "nobody has run the whole thing end to end" possible.
#[cfg(test)]
#[path = "workflow_live_v2_prd_pipeline_tests.rs"]
mod workflow_live_v2_prd_pipeline_tests;

#[path = "workflow_live_v2_script_dry_run.rs"]
mod workflow_live_v2_script_dry_run;
use workflow_live_v2_script_dry_run::*;
// Re-exported: `workflow_live_v2` names this through the subsystem, which the
// splice used to satisfy implicitly.
pub(crate) use workflow_live_v2_script_dry_run::dry_run_workflow_plan;

#[path = "workflow_live_v2_lifecycle.rs"]
mod workflow_live_v2_lifecycle;
use workflow_live_v2_lifecycle::*;

#[path = "workflow_live_v3_orchestrated.rs"]
mod workflow_live_v3_orchestrated;
#[cfg(test)]
use workflow_live_v3_orchestrated::*;

#[path = "workflow_live_v3_author.rs"]
mod workflow_live_v3_author;
#[cfg(test)]
use workflow_live_v3_author::*;

#[path = "workflow_live_v3_author_checks.rs"]
mod workflow_live_v3_author_checks;
use workflow_live_v3_author_checks::*;

#[path = "workflow_live_v2_lifecycle_waves.rs"]
mod workflow_live_v2_lifecycle_waves;

#[path = "workflow_live_v2_lifecycle_impl.rs"]
mod workflow_live_v2_lifecycle_impl;

#[path = "workflow_live_v2_lifecycle_boundary_repair_driver.rs"]
mod workflow_live_v2_lifecycle_boundary_repair_driver;

#[path = "workflow_live_v2_lifecycle_verify.rs"]
mod workflow_live_v2_lifecycle_verify;
#[cfg(test)]
use workflow_live_v2_lifecycle_verify::*;

#[path = "workflow_live_v2_lifecycle_verify_triage.rs"]
mod workflow_live_v2_lifecycle_verify_triage;

#[path = "workflow_live_v2_lifecycle_verify_remediation.rs"]
mod workflow_live_v2_lifecycle_verify_remediation;
use workflow_live_v2_lifecycle_verify_remediation::*;

#[path = "workflow_live_v2_lifecycle_verify_outcome_repair_driver.rs"]
mod workflow_live_v2_lifecycle_verify_outcome_repair_driver;

#[path = "workflow_live_v2_lifecycle_review.rs"]
mod workflow_live_v2_lifecycle_review;

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
#[path = "workflow_live_v2_lifecycle_verify_remediation_tests.rs"]
mod workflow_live_v2_lifecycle_verify_remediation_tests;
#[cfg(test)]
#[path = "workflow_live_v3_boundary_tests.rs"]
mod workflow_live_v3_boundary_tests;
#[cfg(test)]
#[path = "workflow_live_v3_compaction_tests.rs"]
mod workflow_live_v3_compaction_tests;
