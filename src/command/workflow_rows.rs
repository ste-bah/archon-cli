fn deterministic_text(
    label: &str,
    store: &WorkflowStore,
    report: ExecutionReport,
    learning_note: String,
) -> String {
    let evidence_blocks = workflow_status_blocks::evidence_blocks(store, &report.run_id);
    format!(
        "{label} (deterministic CLI smoke mode; pass --live or use TUI /workflow for LLM-backed agents): {} (completed {}, failed {}, skipped {})",
        report.run_id, report.completed, report.failed, report.skipped
    ) + "\n"
        + learning_note.as_str()
        + evidence_blocks.as_str()
}

fn emit_workflow_rows(
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

fn lifecycle(store: &WorkflowStore, run_id: &str, action: LifecycleAction) -> Result<String> {
    let controller = LifecycleController::new(store.clone());
    let v2_restart = generated_v2_restart_target(&action);
    let run = controller.apply(run_id, action)?;
    let invalidated = match v2_restart {
        Some(GeneratedV2RestartTarget::Call(call_id)) => {
            invalidate_generated_v2_call(store, &run, &call_id)?
        }
        Some(GeneratedV2RestartTarget::Item { call_id, item_id }) => {
            invalidate_generated_v2_item(store, &run, &call_id, &item_id)?
        }
        None => Vec::new(),
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
