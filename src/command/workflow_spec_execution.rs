pub(crate) fn load_spec_file(cwd: &Path, path: &str) -> Result<WorkflowSpec> {
    let path = resolve_input_path(cwd, path);
    let raw = fs::read_to_string(&path)?;
    WorkflowSpec::from_yaml(&raw).map_err(Into::into)
}

pub(crate) struct LoadedWorkflowTemplate {
    pub spec: WorkflowSpec,
    pub harness_source: Option<String>,
}

pub(crate) fn load_template(cwd: &Path, name: &str) -> Result<LoadedWorkflowTemplate> {
    if let Some(command) = WorkflowCommandRegistry::project(cwd).load(name)? {
        return Ok(LoadedWorkflowTemplate {
            spec: command.spec,
            harness_source: Some(command.harness_source),
        });
    }
    Ok(LoadedWorkflowTemplate {
        spec: TemplateRegistry::project(cwd).load(name)?.spec,
        harness_source: None,
    })
}

fn repair_workflow(store: &WorkflowStore, run_id: &str) -> Result<String> {
    let run = store.load_state(run_id)?;
    let stage_id = first_repairable_stage(&run).ok_or_else(|| {
        anyhow!("workflow {run_id} has no failed or blocked stage to repair; use /workflow status {run_id}")
    })?;
    let status = lifecycle(
        store,
        run_id,
        LifecycleAction::RestartStage(stage_id.clone()),
    )?;
    Ok(format!(
        "Workflow repair prepared: restarted failed/blocked stage {stage_id}.\nNext: /workflow continue {run_id}\n{status}"
    ))
}

fn restart_task_workflow(store: &WorkflowStore, run_id: &str, task_id: &str) -> Result<String> {
    let run = store.load_state(run_id)?;
    let stage_id = stage_id_for_task(&run, task_id)
        .ok_or_else(|| anyhow!("task '{task_id}' did not match any workflow stage in {run_id}"))?;
    let status = lifecycle(
        store,
        run_id,
        LifecycleAction::RestartStage(stage_id.clone()),
    )?;
    Ok(format!(
        "Workflow task restart prepared: task {task_id} mapped to stage {stage_id}.\nNext: /workflow continue {run_id}\n{status}"
    ))
}

fn first_repairable_stage(run: &WorkflowRun) -> Option<String> {
    run.spec
        .stages
        .iter()
        .find(|stage| {
            run.stages
                .get(&stage.id)
                .is_some_and(|state| state.status == StageStatus::Failed)
        })
        .or_else(|| {
            run.spec.stages.iter().find(|stage| {
                run.stages
                    .get(&stage.id)
                    .is_some_and(|state| state.status == StageStatus::Blocked)
            })
        })
        .map(|stage| stage.id.clone())
}

fn stage_id_for_task(run: &WorkflowRun, task_id: &str) -> Option<String> {
    let aliases = task_aliases(task_id);
    if aliases.is_empty() {
        return None;
    }
    run.spec
        .stages
        .iter()
        .find(|stage| {
            stage_matches_task(&stage.id, &aliases)
                || stage
                    .task
                    .as_deref()
                    .is_some_and(|task| stage_matches_task(task, &aliases))
                || stage_matches_task(&stage.input.to_string(), &aliases)
        })
        .map(|stage| stage.id.clone())
}

fn stage_matches_task(value: &str, aliases: &[String]) -> bool {
    let normalized = normalize_task_token(value);
    let compact = normalized.replace(' ', "");
    aliases.iter().any(|alias| {
        normalized.split_whitespace().any(|token| token == alias)
            || normalized.contains(alias)
            || compact.contains(&alias.replace(' ', ""))
    })
}

fn task_aliases(task_id: &str) -> Vec<String> {
    let normalized = normalize_task_token(task_id);
    let compact = normalized.replace(' ', "");
    let mut aliases = Vec::new();
    push_alias(&mut aliases, normalized);
    push_alias(&mut aliases, compact.clone());
    let digits = compact
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if !digits.is_empty() {
        push_alias(&mut aliases, digits.clone());
        push_alias(&mut aliases, format!("t{digits}"));
    }
    aliases
}

fn push_alias(aliases: &mut Vec<String>, alias: String) {
    if !alias.is_empty() && !aliases.contains(&alias) {
        aliases.push(alias);
    }
}

fn normalize_task_token(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
