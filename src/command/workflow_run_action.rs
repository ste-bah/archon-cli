use super::*;

pub(crate) fn run_action(cwd: &Path, action: CommandAction) -> Result<String> {
    run_action_authorized(cwd, action, None)
}

pub(crate) fn run_action_authorized(
    cwd: &Path,
    action: CommandAction,
    owner_identity: Option<&str>,
) -> Result<String> {
    let store = WorkflowStore::project(cwd);
    let planner = HeuristicWorkflowPlanner;
    let text = match action {
        CommandAction::Plan { task } => planner.plan(&task)?.to_yaml()?,
        CommandAction::PlanSpec { path } => load_spec_file(cwd, &path)?.to_yaml()?,
        CommandAction::Run { .. }
        | CommandAction::RunSpec { .. }
        | CommandAction::RunTemplate { .. }
        | CommandAction::Resume { .. }
        | CommandAction::Continue { .. } => {
            return Err(anyhow!(
                "legacy deterministic workflow execution was removed by the workflow runtime                  rescue; workflows run through the live V2 runtime"
            ));
        }
        CommandAction::Status { run_id } => {
            crate::command::workflow_decompose_owner::require_owner(
                &store,
                &run_id,
                owner_identity,
            )?;
            status_detail_text(&store, &run_id)?
        }
        CommandAction::Repair { run_id } => repair_workflow(&store, &run_id)?,
        CommandAction::Pause { run_id } => {
            crate::command::workflow_decompose_owner::require_owner(
                &store,
                &run_id,
                owner_identity,
            )?;
            lifecycle(&store, &run_id, LifecycleAction::Pause)?
        }
        CommandAction::Cancel { run_id } => {
            crate::command::workflow_decompose_owner::require_owner(
                &store,
                &run_id,
                owner_identity,
            )?;
            lifecycle(&store, &run_id, LifecycleAction::Cancel)?
        }
        CommandAction::ApproveRunOnce { run_id } => {
            approval(&store, cwd, &run_id, ApprovalCommand::RunOnce)?
        }
        CommandAction::ApproveAlways { run_id } => {
            approval(&store, cwd, &run_id, ApprovalCommand::Always)?
        }
        CommandAction::DenyWorkflow { run_id } => {
            approval(&store, cwd, &run_id, ApprovalCommand::Deny)?
        }
        CommandAction::RestartAgent {
            run_id,
            stage_id,
            item,
        } => match item {
            Some(item_id) => lifecycle(
                &store,
                &run_id,
                LifecycleAction::RestartItem { stage_id, item_id },
            )?,
            None => lifecycle(&store, &run_id, LifecycleAction::RestartStage(stage_id))?,
        },
        CommandAction::RestartStage { run_id, stage_id } => {
            lifecycle(&store, &run_id, LifecycleAction::RestartStage(stage_id))?
        }
        CommandAction::RestartTask { run_id, task_id } => {
            restart_task_workflow(&store, &run_id, &task_id)?
        }
        CommandAction::ForceAccept {
            run_id,
            stage_id,
            rationale,
        } => lifecycle(
            &store,
            &run_id,
            LifecycleAction::ForceAcceptStage {
                stage_id,
                forced_by: "workflow-command".to_string(),
                rationale,
                source: "cli_or_tui".to_string(),
            },
        )?,
        CommandAction::Save { run_id, name } => {
            let run = store.load_state(&run_id)?;
            let command = WorkflowCommandRegistry::project(cwd).save_run(&name, &store, &run)?;
            format!(
                "Workflow command saved: {} ({})",
                command.name,
                command.command_dir.display()
            )
        }
        CommandAction::List => list_text(&store)?,
    };
    Ok(text)
}
