use super::*;

pub(super) fn mode(live: bool) -> CliExecutionMode {
    if live {
        CliExecutionMode::Live
    } else {
        CliExecutionMode::Deterministic
    }
}

pub(super) fn run_cli_action(
    spec_file: Option<&PathBuf>,
    from_template: Option<&String>,
    task: &[String],
    decomposed: bool,
) -> Result<CommandAction> {
    let task_counts_as_task = !task.is_empty() && from_template.is_none();
    let selected =
        spec_file.is_some() as u8 + from_template.is_some() as u8 + task_counts_as_task as u8;
    if selected > 1 {
        return Err(anyhow!(
            "use exactly one of task text, --spec-file, or --from-template"
        ));
    }
    if let Some(path) = spec_file {
        return Ok(CommandAction::RunSpec {
            path: path.display().to_string(),
        });
    }
    if let Some(name) = from_template {
        return Ok(CommandAction::RunTemplate {
            name: name.clone(),
            args: template_args_from_task(task)?,
        });
    }
    Ok(CommandAction::Run {
        task: task_string(task)?,
        decomposed,
    })
}

pub(super) fn ensure_resume_from_compatible(
    spec_file: &Option<PathBuf>,
    from_template: &Option<String>,
    decomposed: bool,
) -> Result<()> {
    if spec_file.is_some() || from_template.is_some() || decomposed {
        return Err(anyhow!(
            "--resume-from cannot be combined with --spec-file, --from-template, or --decomposed"
        ));
    }
    Ok(())
}

fn template_args_from_task(task: &[String]) -> Result<Option<serde_json::Value>> {
    if task.is_empty() {
        return Ok(None);
    }
    let raw = task_string(task)?;
    Ok(Some(
        serde_json::from_str(&raw).unwrap_or(serde_json::Value::String(raw)),
    ))
}

pub(super) fn ensure_no_task(task: &[String], flag: &str) -> Result<()> {
    if task.is_empty() {
        Ok(())
    } else {
        Err(anyhow!("{flag} cannot be combined with task text"))
    }
}

pub(super) fn task_string(parts: &[String]) -> Result<String> {
    let task = parts.join(" ");
    if task.trim().is_empty() {
        return Err(anyhow!("workflow task is required"));
    }
    Ok(task)
}

pub(super) fn require_live_approval(live: bool, yes: bool, command: &str) -> Result<()> {
    if live && !yes {
        return Err(anyhow!(
            "{command} requires --yes in non-interactive CLI mode so the generated workflow is explicitly approved"
        ));
    }
    Ok(())
}

pub(super) fn resolve_input_path(cwd: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
