use std::sync::{Arc, Mutex as StdMutex};
#[cfg(test)]
use std::time::Duration;

use archon_workflow::{
    WorkflowError, WorkflowEventKind, WorkflowEventLog, WorkflowStore, WorkflowUiEvent,
    WorkflowV2AgentAdapter, WorkflowV2CallExecution, WorkflowV2CallRecord, WorkflowV2Checkpoint,
    WorkflowV2Evidence, WorkflowV2EvidenceKind, WorkflowV2HostCall, WorkflowV2HostMethod,
    WorkflowV2ResidualGap, WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
    workflow_scaffold_hash,
};
// Only this subsystem's tests build the call/coverage shapes by hand; the host
// itself now receives them already parsed from `archon_workflow::v2::script`.
#[cfg(test)]
use archon_workflow::{
    WorkflowV2HostOptions, WorkflowV2TaskCompletionEvidence, WorkflowV2TaskCoverageStatus,
    WorkflowV2WriteMode,
};
use rquickjs::function::{Async, Func};
use rquickjs::{AsyncContext, AsyncRuntime, CatchResultExt, Promise};
use tokio::sync::Mutex;

use super::WorkflowV2ScriptRuntime;
use super::execute_v2_live_call;
use super::workflow_live_v2_client::LiveV2AgentClient;
use archon_workflow::poll_v2_run_control;
use archon_workflow::task_universe::WorkflowV2TaskUniverse;
use archon_workflow::v2::run_state_sync::mark_v2_call_running;
use archon_workflow::v2::source_graph::{
    complete_source_task_graph, dynamic_wave_source_metadata, input_hash_with_source_fingerprint,
};

// The native lifecycle still consumes the port marker through this module.
use archon_workflow::TERMINAL_HOST_CALL_MARKER;

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
    host_command_executor:
        Option<Arc<dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor>>,
    raw_outcomes_allowed: bool,
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
    pending_calls: workflow_live_v2_script_host_pending::PendingCalls,
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
            host_command_executor: None,
            raw_outcomes_allowed: false,
            reexecuted_task_closure: Arc::new(StdMutex::new(Default::default())),
            pending_calls: Arc::default(),
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

    pub(super) fn with_host_command_executor(
        mut self,
        executor: Arc<dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor>,
    ) -> Self {
        self.host_command_executor = Some(executor);
        self
    }

    pub(super) fn with_raw_outcomes(mut self, allowed: bool) -> Self {
        self.raw_outcomes_allowed = allowed;
        self
    }

    pub(super) async fn run(
        self,
        harness_source: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        let harness_source = harness_source.to_string();
        let author_session = archon_workflow::v2::repair_session::author_current();
        tokio::task::spawn_blocking(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|err| {
                    WorkflowError::SpecInvalid(format!(
                        "workflow.js local async runtime failed: {err}"
                    ))
                })?;
            runtime.block_on(Box::pin(
                archon_workflow::v2::repair_session::inherit_author(
                    author_session,
                    Box::pin(self.run_on_current_thread(&harness_source)),
                ),
            ))
        })
        .await
        .map_err(|err| WorkflowError::SpecInvalid(format!("workflow.js task failed: {err}")))?
    }

    async fn run_on_current_thread(
        mut self,
        harness_source: &str,
    ) -> archon_workflow::WorkflowResult<WorkflowV2ScriptSummary> {
        // Issue-253: the generation a later control outcome must be past.
        let start = observe_start(&self.workflow_store, &self.run_id)?;
        self.initialize_repository_audit().await?;
        let script_args = self.script_args.clone();
        let host = Arc::new(WorkflowScriptHost {
            scaffold_hash: workflow_scaffold_hash(harness_source),
            envelope_shape: script_envelope_shape(harness_source),
            runner: self,
            accumulator: Arc::new(Mutex::new(WorkflowScriptAccumulator {
                script_driven: true,
                ..WorkflowScriptAccumulator::default()
            })),
            tool_host: std::sync::OnceLock::new(),
            tool_budget: Arc::new(std::sync::Mutex::new(Default::default())),
        });
        // Issue-213 C5: record any call a previous host process died under.
        host.record_orphaned_calls();
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
        let watchdog_for_deadline = watchdog.clone();
        // A notification failure the HOST raised, recorded here so the
        // outcome never depends on text a script can write.
        let notification_failure: Arc<StdMutex<Option<String>>> = Arc::default();
        let notification_for_js = notification_failure.clone();
        let host_control: Arc<StdMutex<Option<HostControlStop>>> = Arc::default();
        let control_for_js = host_control.clone();
        let js_result = context
            .async_with(async move |ctx| {
                ctx.globals().set(
                    "__archonHost",
                    Func::from(Async(move |method: String, payload: String| {
                        let host = host_for_js.clone();
                        let watchdog = watchdog_for_js.clone();
                        let notification = notification_for_js.clone();
                        let control = control_for_js.clone();
                        Box::pin(async move {
                            watchdog.pause();
                            let result = Box::pin(host.execute(method, payload)).await;
                            // Issue-285: refused calls are instant, so after a
                            // terminal stop one budget runs without resets.
                            if host.accumulator.lock().await.terminal_host_stop {
                                watchdog.start_terminal_budget();
                            }
                            watchdog.resume();
                            if let Err(WorkflowError::NotificationDelivery(message)) = &result
                                && let Ok(mut slot) = notification.lock()
                            {
                                slot.get_or_insert_with(|| message.clone());
                            }
                            result.or_else(|err| {
                                // Issue-253: run control resolves to a typed
                                // envelope; every other error rejects as before.
                                control_envelope(
                                    &host.runner.workflow_store,
                                    &host.runner.run_id,
                                    &err,
                                )
                                .map(|(envelope, observed)| {
                                    if let Ok(mut slot) = control.lock() {
                                        slot.get_or_insert(observed);
                                    }
                                    envelope
                                })
                                .ok_or_else(|| {
                                    rquickjs::Error::new_from_js_message(
                                        "archon workflow host",
                                        "string",
                                        err.to_string(),
                                    )
                                })
                            })
                        })
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
                // Issue-285: a script idle forever after a terminal stop runs
                // no JavaScript to interrupt, so the budget also ends the wait.
                let settled = tokio::select! {
                    biased;
                    settled = promise.into_future::<String>() => settled,
                    () = watchdog_for_deadline.terminal_budget_spent() => {
                        return Ok(Err(format!(
                            "workflow.js did not settle within {WORKFLOW_JS_WATCHDOG:?} of the host's terminal stop"
                        )));
                    }
                };
                Ok(match settled.catch(&ctx) {
                    Ok(result) => Ok(result),
                    Err(err) => Err(rquickjs::Error::new_from_js_message(
                        "workflow.js",
                        "string",
                        err.to_string(),
                    )
                    .to_string()),
                })
            })
            .await;
        // Covers the deadline, CPU interruption and scripts that return while
        // siblings are pending. No future is polled once `async_with` has
        // returned, so no sibling can publish while this snapshot is taken.
        let terminal_stop = host.accumulator.lock().await.terminal_host_stop;
        if terminal_stop {
            host.interrupt_terminal_calls().await;
        }
        let outcome = js_result.unwrap_or_else(|err| Err(err.to_string()));
        // A host stop survives a concurrent resume while this script unwinds.
        // Round 7 (#285): after a terminal host stop only a stored operator
        // pause or cancel outranks it; a fence a later lifecycle edit raised
        // in a pending call never forges a control outcome.
        let observed = if terminal_stop {
            None
        } else {
            host_control.lock().ok().and_then(|slot| slot.clone())
        };
        if let Some(control) = control_outcome(
            &host.runner.workflow_store,
            &host.runner.run_id,
            start,
            observed.as_ref(),
            outcome.as_ref().err().map(String::as_str),
        ) {
            return Err(control);
        }
        // Host evidence decides the outcome even if the script returns, catches
        // the rejection, or throws unrelated text. No later audit can replace it.
        // A normal return still reports what the script returned.
        if host.accumulator.lock().await.terminal_host_stop {
            let mut summary = host.summary().await;
            summary.script_result = outcome.ok();
            return Ok(summary);
        }
        match outcome {
            Ok(result) => {
                let mut summary = host.summary().await;
                summary.script_result = Some(result);
                host.runner.finalize_repository_audit(summary).await
            }
            Err(error) => {
                let recorded = notification_failure
                    .lock()
                    .ok()
                    .and_then(|slot| slot.clone());
                if let Some(message) = recorded {
                    return Err(WorkflowError::NotificationDelivery(message));
                }
                let summary = host.mark_script_failure(&error).await;
                Ok(summary)
            }
        }
    }
}

#[path = "workflow_live_v2_script_host_pending.rs"]
mod workflow_live_v2_script_host_pending;

#[path = "workflow_live_v2_script_watchdog.rs"]
mod workflow_live_v2_script_watchdog;
use workflow_live_v2_script_watchdog::{WORKFLOW_JS_WATCHDOG, WorkflowJsWatchdog};

struct WorkflowScriptAccumulator {
    status: WorkflowV2Status,
    completed: usize,
    executed: usize,
    reused: usize,
    calls: Vec<WorkflowV2HostCall>,
    failed_call: Option<String>,
    failed_result_path: Option<String>,
    next_action: Option<String>,
    terminal_host_stop: bool,
    /// Issue-285: a JavaScript script drives this host, so a terminal stop is
    /// sticky and refuses later calls. The native lifecycle driver is host code
    /// and keeps its own host-built fallback report.
    script_driven: bool,
    /// Consecutive calls that failed without ever starting. Run-scoped: the
    /// bound only means anything across calls.
    never_started: NeverStartedStreak,
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
            terminal_host_stop: false,
            script_driven: false,
            never_started: NeverStartedStreak::default(),
        }
    }
}

impl WorkflowScriptAccumulator {
    /// A trusted terminal stop that a script can no longer change.
    fn terminal_locked(&self) -> bool {
        self.terminal_host_stop && self.script_driven
    }
}

#[path = "workflow_live_v2_script_control.rs"]
mod workflow_live_v2_script_control;
use workflow_live_v2_script_control::{
    HostControlStop, control_envelope, control_outcome, observe_start,
};

#[path = "workflow_live_v2_script_host.rs"]
mod workflow_live_v2_script_host;
use workflow_live_v2_script_host::*;

use archon_workflow::v2::host_fault::{
    NeverStartedStreak, is_never_started_fault, result_reports_never_started,
    v2_result_for_call_error,
};

// The workflow.js script bridge — payload parsing, source composition, the
// result/reuse reduction, the dry-run recorder and the v3 dialect — is
// `archon_workflow::v2::script`. What is left here is the composition root that
// executes against it. Named once, explicitly: this module used to glob six
// siblings into one namespace every child inherited through `use super::*`.
use archon_workflow::v2::script::{
    ScriptEnvelopeShape, ScriptHostRequest, V3_AUTHOR_BOOTSTRAP, completion_evidence_from_result,
    compose_author_brief, evidence_snapshot_hash, failed_v2_result,
    frontier_resume_record_reusable, is_reusable_status, mark_unresolved_dependency_metadata,
    merge_v2_status, next_action_for_terminal_call, normalize_and_attach_review_findings,
    parse_host_command_request, parse_script_options, record_tasks_all_completed,
    render_author_waves, result_view_json_shaped, reusable_record_has_required_completion_evidence,
    run_terminal_status_contribution, sanitize_v2_gap_id, script_envelope_shape, script_source,
    terminal_stop_for_call, v3_call_family, validate_authored_draft, validate_authored_plan,
    validate_authored_task_accounting, validate_authored_workflow_source,
    validate_executed_acceptance_stage, validate_map_reduce_review_calls,
    validate_review_accounting_from_reducers,
};
#[cfg(test)]
use archon_workflow::v2::script::{normalize_result_for_call, normalize_workflow_export};

// Whole-pipeline plan generation over the real 17-task PRD fixture. It lives
// inside this subsystem because that is the only scope from which the planner,
// the task universe, the scheduler primitives and the per-task review item
// builder are all reachable at once — which is exactly the property that made
// "nobody has run the whole thing end to end" possible.
#[cfg(test)]
#[path = "workflow_live_v2_prd_pipeline_tests.rs"]
mod workflow_live_v2_prd_pipeline_tests;

use archon_workflow::v2::script::dry_run_workflow_plan_full_details;
#[cfg(test)]
use archon_workflow::v2::script::{dry_run_workflow_plan, dry_run_workflow_plan_details};

// Composition root for `archon_workflow::v2::lifecycle_driver`: the only code
// left here that touches the concrete script host.
#[path = "workflow_live_v2_lifecycle.rs"]
mod workflow_live_v2_lifecycle;

// Host side of `archon_workflow::lifecycle_host_port`. Outside the `workflow_*`
// prefix on purpose — see the file's module doc.
#[path = "lifecycle_script_host.rs"]
mod lifecycle_script_host;

// Composition root for the v3 authored-script lifecycle: the only code left
// here that runs the concrete script host over the authoring bootstrap.
#[path = "workflow_live_v3_author.rs"]
mod workflow_live_v3_author;

#[cfg(test)]
#[path = "workflow_live_v2_script_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "workflow_live_v2_script_control_forgery_tests.rs"]
mod workflow_live_v2_script_control_forgery_tests;
#[cfg(test)]
#[path = "workflow_live_v2_script_control_tests.rs"]
mod workflow_live_v2_script_control_tests;
#[cfg(test)]
#[path = "workflow_live_v2_script_pause_rerun_tests.rs"]
mod workflow_live_v2_script_pause_rerun_tests;
// End-to-end lifecycle coverage stays here: it drives the real
// `LiveV2AgentClient`/`WorkflowScriptHost` stack through the driver's public
// surface, which is exactly what cannot be built from inside archon-workflow.
#[cfg(test)]
#[path = "workflow_live_v2_lifecycle_e2e_tests.rs"]
mod workflow_live_v2_lifecycle_e2e_tests;
#[cfg(test)]
#[path = "workflow_live_v3_compaction_tests.rs"]
mod workflow_live_v3_compaction_tests;

#[path = "workflow_repository_audit.rs"]
mod workflow_repository_audit;
pub(super) use workflow_repository_audit::AuditDispatch;
