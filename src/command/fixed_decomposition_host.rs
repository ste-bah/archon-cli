use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_tui::app::TuiEvent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FixedDecompositionTuiRequest {
    pub(crate) prd_path: PathBuf,
    pub(crate) task_root: PathBuf,
}
#[derive(Clone)]
pub(crate) struct FixedDecompositionTuiOwner {
    pub(super) inner: Arc<Mutex<Option<OwnedExecution>>>,
    pub(super) owner_identity: Arc<String>,
}
impl Default for FixedDecompositionTuiOwner {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            owner_identity: Arc::new(uuid::Uuid::new_v4().to_string()),
        }
    }
}
pub(super) struct OwnedExecution {
    pub(super) project_root: PathBuf,
    pub(super) run_id: Arc<Mutex<Option<String>>>,
    pub(super) cancellation_requested: Arc<AtomicBool>,
    pub(super) handle: tokio::task::JoinHandle<()>,
}
impl FixedDecompositionTuiOwner {
    pub(crate) async fn cancel_and_wait(&self) -> Result<()> {
        let execution = self
            .inner
            .lock()
            .map_err(|_| anyhow!("fixed decomposition owner lock is poisoned"))?
            .take();
        let Some(execution) = execution else {
            return Ok(());
        };
        execution
            .cancellation_requested
            .store(true, Ordering::SeqCst);
        let cancellation = match execution.run_id.lock() {
            Ok(slot) => cancel_owned_run(
                &execution.project_root,
                slot.clone(),
                Some(self.owner_identity.as_str()),
            ),
            Err(_) => Err(anyhow!("fixed decomposition run-id owner lock is poisoned")),
        };
        let joined = execution
            .handle
            .await
            .map_err(|error| anyhow!("fixed decomposition worker join failed: {error}"));
        match (cancellation, joined) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(cancel_error), Err(join_error)) => Err(anyhow!(
                "fixed decomposition cancellation failed ({cancel_error}); worker join also failed ({join_error})"
            )),
        }
    }
}

fn cancel_owned_run(
    project_root: &std::path::Path,
    run_id: Option<String>,
    owner_identity: Option<&str>,
) -> Result<()> {
    let Some(run_id) = run_id else {
        return Ok(());
    };
    let store = archon_workflow::WorkflowStore::project(project_root);
    let Ok(run) = store.load_state(&run_id) else {
        return Ok(());
    };
    if matches!(
        run.status,
        archon_workflow::RunStatus::Completed
            | archon_workflow::RunStatus::Failed
            | archon_workflow::RunStatus::Blocked
    ) {
        return Ok(());
    }
    if run.status != archon_workflow::RunStatus::Cancelled {
        archon_workflow::LifecycleController::new(store.clone())
            .apply(&run_id, archon_workflow::LifecycleAction::Cancel)?;
    }
    if crate::command::workflow_decompose_owner::read(&store, &run_id)?.is_some() {
        crate::command::workflow_decompose_owner::record_action(
            &store,
            &run_id,
            owner_identity,
            "cancel",
        )?;
    }
    Ok(())
}

async fn deliver_terminal(
    sink: &archon_workflow::SharedWorkflowUiSink,
    event: archon_workflow::WorkflowUiEvent,
    project_root: &std::path::Path,
    run_id_slot: &Mutex<Option<String>>,
) -> Result<()> {
    deliver_terminal_with_timeout(
        sink,
        event,
        project_root,
        run_id_slot,
        std::time::Duration::from_secs(5),
    )
    .await
}

pub(super) async fn deliver_terminal_with_timeout(
    sink: &archon_workflow::SharedWorkflowUiSink,
    event: archon_workflow::WorkflowUiEvent,
    project_root: &std::path::Path,
    run_id_slot: &Mutex<Option<String>>,
    timeout: std::time::Duration,
) -> Result<()> {
    match tokio::time::timeout(timeout, sink.emit(event)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            mark_terminal_delivery_deferred(project_root, run_id_slot);
            Err(anyhow!(
                "fixed decomposition terminal TUI delivery failed ({error}); inspect durable status and .decompose.log"
            ))
        }
        Err(_) => {
            mark_terminal_delivery_deferred(project_root, run_id_slot);
            Err(anyhow!(
                "fixed decomposition terminal TUI delivery exceeded its bounded wait; inspect durable status and .decompose.log"
            ))
        }
    }
}

fn mark_terminal_delivery_deferred(
    project_root: &std::path::Path,
    run_id_slot: &Mutex<Option<String>>,
) {
    let run_id = run_id_slot.lock().ok().and_then(|slot| slot.clone());
    let Some(run_id) = run_id else {
        return;
    };
    let store = archon_workflow::WorkflowStore::project(project_root);
    let state_path = store
        .run_dir(&run_id)
        .join(crate::command::workflow_decompose::FIXED_DECOMPOSITION_STATE_PATH);
    let Some(state) = std::fs::read(&state_path).ok().and_then(|bytes| {
        serde_json::from_slice::<archon_workflow::FixedDecompositionStateV1>(&bytes).ok()
    }) else {
        return;
    };
    let _ = crate::command::workflow_decompose_log::append_fixed_log_marker(
        std::path::Path::new(&state.log_path),
        "ui_terminal_delivery_deferred",
        &run_id,
        &state.identity,
    );
}

pub(crate) fn parse_slash_args(args: &[String]) -> Result<FixedDecompositionTuiRequest> {
    if args.first().is_none_or(|value| value != "decompose") {
        return Err(anyhow!("expected workflow decompose arguments"));
    }
    let mut prd = None;
    let mut tasks = None;
    let mut index = 1usize;
    while index < args.len() {
        let flag = &args[index];
        let value = args
            .get(index + 1)
            .ok_or_else(|| anyhow!("/workflow decompose requires a value after {flag}"))?;
        match flag.as_str() {
            "--prd" if prd.is_none() => prd = Some(PathBuf::from(value)),
            "--tasks" if tasks.is_none() => tasks = Some(PathBuf::from(value)),
            "--prd" | "--tasks" => return Err(anyhow!("duplicate {flag}")),
            other => return Err(anyhow!("unknown /workflow decompose argument {other}")),
        }
        index += 2;
    }
    Ok(FixedDecompositionTuiRequest {
        prd_path: prd.ok_or_else(|| anyhow!("/workflow decompose requires --prd <PATH>"))?,
        task_root: tasks.ok_or_else(|| anyhow!("/workflow decompose requires --tasks <DIR>"))?,
    })
}

pub(crate) fn parse_resume_args(args: &[String]) -> Result<Option<String>> {
    if args.first().is_none_or(|value| value != "resume") {
        return Ok(None);
    }
    let values: Vec<&str> = args[1..]
        .iter()
        .map(String::as_str)
        .filter(|value| *value != "--live")
        .collect();
    if values.len() != 1 || values[0].trim().is_empty() {
        return Err(anyhow!("/workflow resume requires exactly one run id"));
    }
    Ok(Some(values[0].to_string()))
}

pub(crate) fn handle_command_context(
    ctx: &mut crate::command::registry::CommandContext,
    args: &[String],
    cwd: PathBuf,
) -> Result<bool> {
    if args.first().is_some_and(|value| value == "decompose") {
        let request = parse_slash_args(args)?;
        let (config, env_vars, owner) = launch_context(ctx)?;
        spawn(cwd, request, config, env_vars, ctx.tui_tx.clone(), owner)?;
        ctx.emit(TuiEvent::SlashCommandComplete);
        return Ok(true);
    }
    if args.first().is_some_and(|value| value == "status") {
        let run_id = parse_single_run_id(args, "status")?;
        if !crate::command::workflow_decompose::is_fixed_decomposition_run(&cwd, &run_id)? {
            return Ok(false);
        }
        let (_, _, owner) = launch_context(ctx)?;
        let store = archon_workflow::WorkflowStore::project(&cwd);
        crate::command::workflow_decompose_owner::require_owner(
            &store,
            &run_id,
            Some(owner.owner_identity.as_str()),
        )?;
        let output = crate::command::workflow::run_action_authorized(
            &cwd,
            archon_workflow::CommandAction::Status {
                run_id: run_id.clone(),
            },
            Some(owner.owner_identity.as_str()),
        )?;
        crate::command::workflow_decompose_owner::record_action(
            &store,
            &run_id,
            Some(owner.owner_identity.as_str()),
            "status",
        )?;
        ctx.emit(TuiEvent::TextDelta(output));
        ctx.emit(TuiEvent::SlashCommandComplete);
        return Ok(true);
    }
    if args.first().is_some_and(|value| value == "cancel") {
        let run_id = parse_single_run_id(args, "cancel")?;
        if !crate::command::workflow_decompose::is_fixed_decomposition_run(&cwd, &run_id)? {
            return Ok(false);
        }
        let (_, _, owner) = launch_context(ctx)?;
        let store = archon_workflow::WorkflowStore::project(&cwd);
        crate::command::workflow_decompose_owner::require_owner(
            &store,
            &run_id,
            Some(owner.owner_identity.as_str()),
        )?;
        archon_workflow::LifecycleController::new(store.clone())
            .apply(&run_id, archon_workflow::LifecycleAction::Cancel)?;
        crate::command::workflow_decompose_owner::record_action(
            &store,
            &run_id,
            Some(owner.owner_identity.as_str()),
            "cancel",
        )?;
        ctx.emit(TuiEvent::TextDelta(format!(
            "Workflow {run_id} cancelled\n"
        )));
        ctx.emit(TuiEvent::SlashCommandComplete);
        return Ok(true);
    }
    if args.first().is_some_and(|value| value == "pause") {
        let run_id = parse_single_run_id(args, "pause")?;
        if !crate::command::workflow_decompose::is_fixed_decomposition_run(&cwd, &run_id)? {
            return Ok(false);
        }
        let (_, _, owner) = launch_context(ctx)?;
        let store = archon_workflow::WorkflowStore::project(&cwd);
        crate::command::workflow_decompose_owner::require_owner(
            &store,
            &run_id,
            Some(owner.owner_identity.as_str()),
        )?;
        let output = crate::command::workflow::run_action_authorized(
            &cwd,
            archon_workflow::CommandAction::Pause {
                run_id: run_id.clone(),
            },
            Some(owner.owner_identity.as_str()),
        )?;
        crate::command::workflow_decompose_owner::record_action(
            &store,
            &run_id,
            Some(owner.owner_identity.as_str()),
            "pause",
        )?;
        ctx.emit(TuiEvent::TextDelta(output));
        ctx.emit(TuiEvent::SlashCommandComplete);
        return Ok(true);
    }
    let Some(run_id) = parse_resume_args(args)? else {
        return Ok(false);
    };
    if !crate::command::workflow_decompose::is_fixed_decomposition_run(&cwd, &run_id)? {
        return Ok(false);
    }
    let (config, env_vars, owner) = launch_context(ctx)?;
    spawn_resume(cwd, run_id, config, env_vars, ctx.tui_tx.clone(), owner)?;
    ctx.emit(TuiEvent::SlashCommandComplete);
    Ok(true)
}

fn parse_single_run_id(args: &[String], action: &str) -> Result<String> {
    if args.len() != 2 || args[1].trim().is_empty() {
        return Err(anyhow!("/workflow {action} requires exactly one run id"));
    }
    Ok(args[1].clone())
}

fn launch_context(
    ctx: &crate::command::registry::CommandContext,
) -> Result<(ArchonConfig, ArchonEnvVars, FixedDecompositionTuiOwner)> {
    let config = ctx.workflow_config.clone().ok_or_else(|| {
        anyhow!("fixed decomposition requires the startup workflow configuration snapshot")
    })?;
    let env_vars = ctx
        .workflow_env_vars
        .clone()
        .ok_or_else(|| anyhow!("fixed decomposition requires the startup environment snapshot"))?;
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            crate::command::workflow_decompose::DECOMPOSE_GATE_OFF_REMEDY
        ));
    }
    let owner = ctx
        .fixed_decomposition_owner
        .clone()
        .ok_or_else(|| anyhow!("fixed decomposition requires retained executor ownership"))?;
    Ok((config, env_vars, owner))
}

pub(crate) fn spawn(
    cwd: PathBuf,
    request: FixedDecompositionTuiRequest,
    config: ArchonConfig,
    env_vars: ArchonEnvVars,
    tui_tx: archon_tui::event_channel::TuiEventSender,
    owner: FixedDecompositionTuiOwner,
) -> Result<()> {
    spawn_owned(
        cwd,
        FixedExecutionRequest::Launch(request),
        config,
        env_vars,
        tui_tx,
        owner,
    )
}

pub(crate) fn spawn_resume(
    cwd: PathBuf,
    run_id: String,
    config: ArchonConfig,
    env_vars: ArchonEnvVars,
    tui_tx: archon_tui::event_channel::TuiEventSender,
    owner: FixedDecompositionTuiOwner,
) -> Result<()> {
    spawn_owned(
        cwd,
        FixedExecutionRequest::Resume(run_id),
        config,
        env_vars,
        tui_tx,
        owner,
    )
}

enum FixedExecutionRequest {
    Launch(FixedDecompositionTuiRequest),
    Resume(String),
}

fn spawn_owned(
    cwd: PathBuf,
    request: FixedExecutionRequest,
    config: ArchonConfig,
    env_vars: ArchonEnvVars,
    tui_tx: archon_tui::event_channel::TuiEventSender,
    owner: FixedDecompositionTuiOwner,
) -> Result<()> {
    let project_root = cwd.clone();
    let project_root_for_worker = project_root.clone();
    let owner_identity = owner.owner_identity.as_str().to_string();
    let mut guard = owner
        .inner
        .lock()
        .map_err(|_| anyhow!("fixed decomposition owner lock is poisoned"))?;
    if guard
        .as_ref()
        .is_some_and(|current| !current.handle.is_finished())
    {
        return Err(anyhow!(
            "a fixed decomposition is already active in this session; inspect it with /workflow status"
        ));
    }
    let retained_run_id = match &request {
        FixedExecutionRequest::Launch(_) => None,
        FixedExecutionRequest::Resume(run_id) => Some(run_id.clone()),
    };
    let run_id_slot = Arc::new(Mutex::new(retained_run_id));
    let run_id_for_worker = Arc::clone(&run_id_slot);
    let cancellation_requested = Arc::new(AtomicBool::new(false));
    let cancellation_for_worker = Arc::clone(&cancellation_requested);
    let handle = archon_observability::spawn_blocking_named("fixed-decomposition-run", move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = tui_tx.send(TuiEvent::Error(format!(
                    "Fixed decomposition runtime failed: {error}"
                )));
                return;
            }
        };
        runtime.block_on(async move {
            let ui_sink = crate::command::tui_workflow_ui_sink::FixedDecompositionWorkflowUiSink::arc(
                tui_tx.clone(),
            );
            let completion_sink =
                crate::command::tui_workflow_ui_sink::TuiWorkflowUiSink::arc(tui_tx.clone());
            let factory = crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
                &config,
                &env_vars,
            );
            let result = match request {
                FixedExecutionRequest::Launch(request) => {
                    crate::command::workflow_decompose::run_fixed_decomposition_with_factory_and_sink(
                        &cwd,
                        &request.prd_path,
                        &request.task_root,
                        true,
                        &config,
                        &env_vars,
                        &factory,
                        ui_sink,
                        Some(run_id_for_worker.as_ref()),
                        Some(owner_identity.as_str()),
                        Some(cancellation_for_worker.as_ref()),
                    )
                    .await
                }
                FixedExecutionRequest::Resume(run_id) => {
                    crate::command::workflow_decompose::resume_fixed_decomposition_with_factory_and_sink(
                        &cwd,
                        &run_id,
                        true,
                        &config,
                        &env_vars,
                        &factory,
                        ui_sink,
                        Some(owner_identity.as_str()),
                        Some(cancellation_for_worker.as_ref()),
                    )
                    .await
                }
            };
            let event = match result {
                Ok(output) => archon_workflow::WorkflowUiEvent::Text(output),
                Err(error) => archon_workflow::WorkflowUiEvent::Error(format!(
                    "Fixed decomposition failed: {error:#}"
                )),
            };
            if let Err(error) = deliver_terminal(
                &completion_sink,
                event,
                &project_root_for_worker,
                run_id_for_worker.as_ref(),
            )
            .await
            {
                tracing::warn!(%error, "fixed decomposition completion delivery failed");
            }
        });
    });
    *guard = Some(OwnedExecution {
        project_root,
        run_id: run_id_slot,
        cancellation_requested,
        handle,
    });
    Ok(())
}
