use super::*;

#[derive(Debug, Clone, Copy)]
pub(super) enum ApprovalCommand {
    RunOnce,
    Always,
    Deny,
}

pub(super) fn approval(
    store: &WorkflowStore,
    cwd: &Path,
    run_id: &str,
    command: ApprovalCommand,
) -> Result<String> {
    let run = store.load_state(run_id)?;
    let approvals = WorkflowApprovalStore::project(cwd);
    let record = match command {
        ApprovalCommand::RunOnce => {
            approvals.approve_run_once(cwd, store, &run, "workflow-command")?
        }
        ApprovalCommand::Always => {
            approvals.approve_always_for_project(cwd, store, &run, "workflow-command")?
        }
        ApprovalCommand::Deny => {
            let record = approvals.deny_run(cwd, store, &run, "workflow-command")?;
            let _ =
                LifecycleController::new(store.clone()).apply(run_id, LifecycleAction::Cancel)?;
            record
        }
    };
    let action = match &record.decision {
        archon_workflow::WorkflowApprovalDecision::RunOnce => "approved once",
        archon_workflow::WorkflowApprovalDecision::AlwaysForProject => "approved always",
        archon_workflow::WorkflowApprovalDecision::Denied => "denied",
    };
    let count_label = if matches!(
        record.origin,
        Some(WorkflowBundleOrigin::GeneratedHarness | WorkflowBundleOrigin::SavedCommand)
    ) {
        "dynamic host calls"
    } else {
        "phases"
    };
    Ok(format!(
        "Workflow {run_id} {action}: {} {count_label}, max_agents={}, max_parallelism={}, {}, raw_script={}",
        record.phase_count,
        record.max_agents,
        record.max_parallelism,
        approval_subject_summary(&record),
        record.raw_script_path
    ))
}

fn approval_subject_summary(record: &archon_workflow::WorkflowApprovalRecord) -> String {
    let generated = record
        .generated_metadata_hash
        .as_deref()
        .map(short_hash)
        .unwrap_or_else(|| "none".to_string());
    format!(
        "approval_subject={}, script={}, compiled={}, generated_metadata={}",
        short_hash(&record.approval_subject_hash),
        short_hash(&record.workflow_hash),
        short_hash(&record.compiled_hash),
        generated
    )
}

pub(super) fn status_text(run: &archon_workflow::WorkflowRun) -> String {
    let accepted = run
        .stages
        .values()
        .filter(|stage| run.accepted_stage(&stage.id))
        .count();
    let failed = run
        .stages
        .values()
        .filter(|stage| matches!(stage.status, archon_workflow::StageStatus::Failed))
        .count();
    let blocked = run
        .stages
        .values()
        .filter(|stage| matches!(stage.status, archon_workflow::StageStatus::Blocked))
        .count();
    let forced = run
        .stages
        .values()
        .filter(|stage| matches!(stage.status, archon_workflow::StageStatus::ForcedAccepted))
        .count();
    let status = match run.status {
        RunStatus::Planned => "planned",
        RunStatus::Running => "running",
        RunStatus::Paused => "paused",
        RunStatus::NeedsReview => "needs_review",
        RunStatus::Blocked => "blocked",
        RunStatus::Failed => "failed",
        RunStatus::Cancelled => "cancelled",
        RunStatus::Completed => "completed",
    };
    format!(
        "Workflow {}: {} ({accepted}/{} accepted, {blocked} blocked, {forced} forced, {failed} failed, current={}, next={})",
        run.id,
        status,
        run.stages.len(),
        visible_stage_summary(run),
        next_workflow_action(run)
    )
}

pub(super) fn visible_stage_summary(run: &WorkflowRun) -> String {
    for status in [
        StageStatus::Running,
        StageStatus::NeedsReview,
        StageStatus::Failed,
        StageStatus::Blocked,
        StageStatus::Pending,
        StageStatus::Paused,
    ] {
        if let Some(stage) = run.spec.stages.iter().find(|stage| {
            run.stages
                .get(&stage.id)
                .is_some_and(|state| state.status == status)
        }) {
            let mut summary = stage.id.clone();
            if let Some(task) = &stage.task {
                summary.push_str(": ");
                summary.push_str(&one_line(task, 90));
            }
            if let Some(error) = run
                .stages
                .get(&stage.id)
                .and_then(|state| state.error.as_ref())
            {
                summary.push_str(" error=");
                summary.push_str(&one_line(error, 90));
            }
            return summary;
        }
    }
    "none".to_string()
}

pub(super) fn next_workflow_action(run: &WorkflowRun) -> String {
    match run.status {
        RunStatus::NeedsReview => format!("/workflow resume --live {}", run.id),
        RunStatus::Failed | RunStatus::Blocked => format!("/workflow repair {}", run.id),
        RunStatus::Paused => format!("/workflow continue {}", run.id),
        RunStatus::Running | RunStatus::Planned => format!("wait or /workflow status {}", run.id),
        RunStatus::Completed => "review final report".to_string(),
        RunStatus::Cancelled => "start a new workflow".to_string(),
    }
}

pub(super) fn status_detail_text(store: &WorkflowStore, run_id: &str) -> Result<String> {
    let run = store.load_state(run_id)?;
    let mut out = status_text(&run);
    out.push('\n');
    out.push_str(&format!(
        "name: {}\ntask: {}\ncreated: {}\nupdated: {}\ngeneration: {}\n",
        run.spec.name, run.spec.task, run.created_at, run.updated_at, run.generation
    ));
    let mut generated_v2_bundle = false;
    match archon_workflow::WorkflowBundle::verify(store, run_id) {
        Ok(manifest) => {
            let generated_v2 = matches!(
                manifest.origin,
                WorkflowBundleOrigin::GeneratedHarness | WorkflowBundleOrigin::SavedCommand
            );
            generated_v2_bundle = generated_v2;
            let count_label = if generated_v2 {
                "dynamic_host_calls"
            } else {
                "phases"
            };
            let compiled_label = if generated_v2 {
                "compiled_metadata"
            } else {
                "workflow.compiled.yaml"
            };
            out.push_str(&format!(
                "bundle: verified workflow.js={} {}={} {}={} max_agents={} max_parallelism={} write_capable={}\n",
                short_hash(&manifest.workflow_hash),
                compiled_label,
                short_hash(&manifest.compiled_hash),
                count_label,
                manifest.phase_count,
                manifest.max_agents,
                manifest.max_parallelism,
                display_list(&manifest.write_capable_stages)
            ));
        }
        Err(err) => {
            out.push_str(&format!("bundle: not verified ({err})\n"));
        }
    }

    if generated_v2_bundle {
        out.push_str("\ndynamic host-call metadata:\n");
    } else {
        out.push_str("\nphases/stages:\n");
    }
    for stage in &run.spec.stages {
        let state = run.stages.get(&stage.id);
        let status = state
            .map(|state| format!("{:?}", state.status).to_ascii_lowercase())
            .unwrap_or_else(|| "missing".to_string());
        let attempts = state.map_or(0, |state| state.attempt);
        let artifact_count = state.map_or(0, |state| state.artifacts.len());
        out.push_str(&format!(
            "- {} kind={:?} status={} attempts={} depends_on={} artifacts={}",
            stage.id,
            stage.kind,
            status,
            attempts,
            display_list(&stage.depends_on),
            artifact_count
        ));
        if let Some(tier) = stage.provider_tier {
            out.push_str(&format!(" tier={tier:?}"));
        }
        if let Some(item_kind) = stage.item_kind {
            out.push_str(&format!(" item_kind={item_kind:?}"));
        }
        if let Some(max_parallelism) = stage.max_parallelism {
            out.push_str(&format!(" max_parallelism={max_parallelism}"));
        }
        if !stage.expected_target_files.is_empty() {
            out.push_str(&format!(
                " target_files={}",
                display_list(&stage.expected_target_files)
            ));
        }
        if let Some(command) = &stage.verify_command {
            out.push_str(&format!(" verify_command={}", one_line(command, 140)));
        }
        if let Some(state) = state {
            if let Some(started_at) = state.started_at {
                out.push_str(&format!(" started={started_at}"));
            }
            if let Some(completed_at) = state.completed_at {
                out.push_str(&format!(" completed={completed_at}"));
            }
            if let Some(error) = &state.error {
                out.push_str(&format!(" error={}", one_line(error, 180)));
            }
        }
        out.push('\n');
    }

    if !run.items.is_empty() {
        out.push_str("\nitems:\n");
        for item in run.items.values() {
            out.push_str(&format!(
                "- {} stage={} status={:?}",
                item.id, item.stage_id, item.status
            ));
            if let Some(artifact) = &item.artifact {
                out.push_str(&format!(" artifact={}", artifact.id));
            }
            if let Some(error) = &item.error {
                out.push_str(&format!(" error={}", one_line(error, 160)));
            }
            out.push('\n');
        }
    }

    let prompts = prompt_previews(store, run_id)?;
    if !prompts.is_empty() {
        out.push_str("\nprompts:\n");
        for prompt in prompts {
            out.push_str(&format!(
                "- stage={} item={} input_hash={} prompt_hash={} created={} path={}\n",
                prompt.stage_id,
                prompt.item_id,
                short_hash(&prompt.input_hash),
                short_hash(&prompt.prompt_hash),
                prompt.created_at.unwrap_or_else(|| "unknown".to_string()),
                prompt.path.display()
            ));
        }
    }

    if let Ok(detail) = archon_workflow::web_api::detail(store, run_id) {
        if !detail.agents.is_empty() {
            out.push_str("\nagent outputs:\n");
            for agent in detail.agents {
                out.push_str(&format!(
                    "- stage={} item={} status={} provider={} model={} tokens={}/{} cost=${:.6} output={}",
                    agent.stage_id,
                    agent.item_id,
                    agent.status,
                    agent.provider.unwrap_or_else(|| "unknown".to_string()),
                    agent.model.unwrap_or_else(|| "unknown".to_string()),
                    agent.tokens_in,
                    agent.tokens_out,
                    agent.cost_usd,
                    agent.output_path
                ));
                if let Some(artifact_id) = agent.artifact_id {
                    out.push_str(&format!(" artifact={artifact_id}"));
                }
                if let Some(error) = agent.error {
                    out.push_str(&format!(" error={}", one_line(&error, 160)));
                }
                if let Some(preview) = agent.result_preview {
                    out.push_str(&format!(" preview={}", one_line(&preview, 180)));
                }
                out.push('\n');
            }
        }
        if !detail.artifacts.is_empty() {
            out.push_str("\nartifacts:\n");
            for artifact in detail.artifacts {
                out.push_str(&format!(
                    "- {} stage={} path={} hash={}\n",
                    artifact.id,
                    artifact.producing_stage,
                    artifact.path,
                    short_hash(&artifact.content_hash)
                ));
            }
        }
        if !detail.events.is_empty() {
            out.push_str("\nrecent events:\n");
            for event in detail.events.into_iter().take(8) {
                out.push_str(&format!(
                    "- #{} {} {} {}\n",
                    event.seq,
                    event.created_at,
                    event.status,
                    one_line(&event.summary, 120)
                ));
            }
        }
    }
    Ok(out)
}

#[derive(Debug)]
struct PromptPreview {
    stage_id: String,
    item_id: String,
    input_hash: String,
    prompt_hash: String,
    created_at: Option<String>,
    path: PathBuf,
}

fn prompt_previews(store: &WorkflowStore, run_id: &str) -> Result<Vec<PromptPreview>> {
    let root = store.run_dir(run_id).join("prompts");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    collect_json_files(&root, &mut paths)?;
    let mut previews = Vec::new();
    for path in paths {
        let raw = fs::read_to_string(&path)?;
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        previews.push(PromptPreview {
            stage_id: string_json_field(&value, "stage_id").unwrap_or_default(),
            item_id: string_json_field(&value, "item_id").unwrap_or_default(),
            input_hash: string_json_field(&value, "input_hash").unwrap_or_default(),
            prompt_hash: string_json_field(&value, "prompt_hash").unwrap_or_default(),
            created_at: string_json_field(&value, "created_at"),
            path: path
                .strip_prefix(store.run_dir(run_id))
                .unwrap_or(path.as_path())
                .to_path_buf(),
        });
    }
    previews.sort_by(|a, b| {
        a.stage_id
            .cmp(&b.stage_id)
            .then_with(|| a.item_id.cmp(&b.item_id))
    });
    Ok(previews)
}

fn collect_json_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            out.push(path);
        }
    }
    Ok(())
}

fn string_json_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn display_list(values: &[String]) -> String {
    if values.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", values.join(", "))
    }
}

fn one_line(value: &str, max_chars: usize) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        compact
    } else {
        let preview = compact.chars().take(max_chars).collect::<String>();
        format!("{preview}...")
    }
}

fn short_hash(value: &str) -> String {
    value.chars().take(12).collect()
}

pub(super) fn list_text(store: &WorkflowStore) -> Result<String> {
    let runs = store.list_runs()?;
    if runs.is_empty() {
        return Ok("No workflow runs found.".to_string());
    }
    Ok(runs.iter().map(status_text).collect::<Vec<_>>().join("\n"))
}
