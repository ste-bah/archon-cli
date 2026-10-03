use super::*;

pub(super) fn emit_workflow_rows(
    cwd: &Path,
    action: &CommandAction,
    ctx: &mut CommandContext,
) -> Result<bool> {
    let store = WorkflowStore::project(cwd);
    let rows = match action {
        CommandAction::List => store
            .list_runs()?
            .iter()
            .map(run_row)
            .collect::<Vec<EvidenceRowPayload>>(),
        CommandAction::Status { run_id } => {
            let run = store.load_state(run_id)?;
            run.stages
                .values()
                .map(|stage| EvidenceRowPayload {
                    id: stage.id.clone(),
                    title: stage.id.clone(),
                    status: format!("{:?}", stage.status).to_ascii_lowercase(),
                    detail: format!(
                        "attempts={} artifacts={}{}",
                        stage.attempt,
                        stage.artifacts.len(),
                        stage
                            .error
                            .as_ref()
                            .map(|error| format!(" error={error}"))
                            .unwrap_or_default()
                    ),
                })
                .collect()
        }
        _ => return Ok(false),
    };
    ctx.emit(TuiEvent::OpenViewRows {
        view_id: ViewId::Workflow,
        rows,
    });
    Ok(true)
}

fn run_row(run: &archon_workflow::WorkflowRun) -> EvidenceRowPayload {
    let accepted = run
        .stages
        .values()
        .filter(|stage| run.accepted_stage(&stage.id))
        .count();
    let blocked = run
        .stages
        .values()
        .filter(|stage| matches!(stage.status, archon_workflow::StageStatus::Blocked))
        .count();
    let failed = run
        .stages
        .values()
        .filter(|stage| matches!(stage.status, archon_workflow::StageStatus::Failed))
        .count();
    EvidenceRowPayload {
        id: run.id.clone(),
        title: run.spec.name.clone(),
        status: format!("{:?}", run.status).to_ascii_lowercase(),
        detail: format!(
            "{accepted}/{} accepted, {blocked} blocked, {failed} failed, current={}, next={}",
            run.stages.len(),
            visible_stage_summary(run),
            next_workflow_action(run)
        ),
    }
}

pub(super) fn lifecycle(
    store: &WorkflowStore,
    run_id: &str,
    action: LifecycleAction,
) -> Result<String> {
    if is_restart(&action) {
        return with_restart_lease(store, run_id, || lifecycle_unleased(store, run_id, action));
    }
    lifecycle_unleased(store, run_id, action)
}

fn is_restart(action: &LifecycleAction) -> bool {
    matches!(
        action,
        LifecycleAction::RestartStage(_) | LifecycleAction::RestartItem { .. }
    )
}

/// [`lifecycle`] for a caller that already holds the restart lease
/// (`with_restart_lease`), or for an action that needs none. A restart
/// rewinds the state and invalidates the V2 cache as one commit under the
/// run lock (Issue-267).
pub(super) fn lifecycle_unleased(
    store: &WorkflowStore,
    run_id: &str,
    action: LifecycleAction,
) -> Result<String> {
    let controller = LifecycleController::new(store.clone());
    let (run, invalidated) = if is_restart(&action) {
        controller.apply_restart(run_id, action)?
    } else {
        (controller.apply(run_id, action)?, Vec::new())
    };
    let run = store.load_state(&run.id).unwrap_or(run);
    let mut output = status_text(&run);
    if !invalidated.is_empty() {
        output.push_str(&format!(
            "\nV2 resume cache invalidated for {} call(s): {}",
            invalidated.len(),
            invalidated.join(", ")
        ));
    }
    Ok(output)
}
