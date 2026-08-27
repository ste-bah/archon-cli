//! Interactive host delegation for fixed decomposition launch.

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

#[derive(Clone, Default)]
pub(crate) struct FixedDecompositionTuiOwner {
    pub(super) inner: Arc<Mutex<Option<OwnedExecution>>>,
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
        let run_id = execution
            .run_id
            .lock()
            .map_err(|_| anyhow!("fixed decomposition run-id owner lock is poisoned"))?
            .clone();
        if let Some(run_id) = run_id {
            let store = archon_workflow::WorkflowStore::project(&execution.project_root);
            if let Ok(run) = store.load_state(&run_id)
                && !matches!(
                    run.status,
                    archon_workflow::RunStatus::Completed
                        | archon_workflow::RunStatus::Cancelled
                        | archon_workflow::RunStatus::Failed
                        | archon_workflow::RunStatus::Blocked
                )
            {
                archon_workflow::LifecycleController::new(store)
                    .apply(&run_id, archon_workflow::LifecycleAction::Cancel)?;
            }
        }
        execution
            .handle
            .await
            .map_err(|error| anyhow!("fixed decomposition worker join failed: {error}"))
    }
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

pub(crate) fn handle_command_context(
    ctx: &mut crate::command::registry::CommandContext,
    args: &[String],
    cwd: PathBuf,
) -> Result<bool> {
    if args.first().is_none_or(|value| value != "decompose") {
        return Ok(false);
    }
    let request = parse_slash_args(args)?;
    let config = ctx.workflow_config.clone().ok_or_else(|| {
        anyhow!("/workflow decompose requires the startup workflow configuration snapshot")
    })?;
    let env_vars = ctx
        .workflow_env_vars
        .clone()
        .ok_or_else(|| anyhow!("/workflow decompose requires the startup environment snapshot"))?;
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            crate::command::workflow_decompose::DECOMPOSE_GATE_OFF_REMEDY
        ));
    }
    let owner = ctx
        .fixed_decomposition_owner
        .clone()
        .ok_or_else(|| anyhow!("/workflow decompose requires retained executor ownership"))?;
    spawn(cwd, request, config, env_vars, ctx.tui_tx.clone(), owner)?;
    ctx.emit(TuiEvent::SlashCommandComplete);
    Ok(true)
}

pub(crate) fn spawn(
    cwd: PathBuf,
    request: FixedDecompositionTuiRequest,
    config: ArchonConfig,
    env_vars: ArchonEnvVars,
    tui_tx: archon_tui::event_channel::TuiEventSender,
    owner: FixedDecompositionTuiOwner,
) -> Result<()> {
    let project_root = cwd.clone();
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
    let run_id_slot = Arc::new(Mutex::new(None));
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
            let ui_sink = archon_workflow::ui_sink_port::ResilientWorkflowUiSink::wrap(
                crate::command::tui_workflow_ui_sink::TuiWorkflowUiSink::arc(tui_tx.clone()),
            );
            let factory = crate::command::pipeline_workflow_llm::SubagentPipelineClientFactory::configured_only(
                &config,
                &env_vars,
            );
            let result = crate::command::workflow_decompose::run_fixed_decomposition_with_factory_and_sink(
                &cwd,
                &request.prd_path,
                &request.task_root,
                true,
                &config,
                &env_vars,
                &factory,
                ui_sink,
                Some(run_id_for_worker.as_ref()),
                Some(cancellation_for_worker.as_ref()),
            )
            .await;
            let event = match result {
                Ok(output) => TuiEvent::TextDelta(output),
                Err(error) => TuiEvent::Error(format!("Fixed decomposition failed: {error:#}")),
            };
            if let Err(error) = tui_tx.send_async(event).await {
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
