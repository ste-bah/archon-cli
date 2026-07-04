fn v2_result_views(
    store: &WorkflowStore,
    run_id: &str,
) -> WorkflowResult<Vec<WorkflowV2ResultView>> {
    let root = store.run_dir(run_id).join("v2");
    let results_root = root.join("results");
    if !results_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&results_root).map_err(|e| WorkflowError::io(&results_root, e))? {
        let entry = entry.map_err(|e| WorkflowError::io(&results_root, e))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        if contains_forbidden_field(&value) {
            continue;
        }
        let clean = sanitize_value(value);
        let call_id = clean
            .get("call")
            .and_then(|call| string_field(call, "id"))
            .unwrap_or_else(|| "unknown".to_string());
        let result = clean.get("result").unwrap_or(&serde_json::Value::Null);
        out.push(WorkflowV2ResultView {
            call_id: call_id.clone(),
            status: string_field(&clean, "status").unwrap_or_else(|| "unknown".to_string()),
            summary: string_field(result, "summary").unwrap_or_default(),
            result_path: path
                .strip_prefix(&root)
                .unwrap_or(path.as_path())
                .display()
                .to_string(),
            artifact_count: result
                .get("artifacts")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len),
            branch_count: branch_count_for_call(&root, &call_id),
        });
    }
    out.sort_by(|left, right| left.call_id.cmp(&right.call_id));
    Ok(out)
}

fn v2_branch_views(
    store: &WorkflowStore,
    run_id: &str,
) -> WorkflowResult<Vec<WorkflowV2BranchView>> {
    let root = store.run_dir(run_id).join("v2");
    let branches_root = root.join("branches");
    if !branches_root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    collect_v2_branch_views(&root, &branches_root, &mut out)?;
    out.sort_by(|left, right| {
        left.call_id
            .cmp(&right.call_id)
            .then_with(|| left.item_id.cmp(&right.item_id))
    });
    Ok(out)
}

fn collect_v2_branch_views(
    root: &Path,
    branches_root: &Path,
    out: &mut Vec<WorkflowV2BranchView>,
) -> WorkflowResult<()> {
    for call_entry in
        fs::read_dir(branches_root).map_err(|e| WorkflowError::io(branches_root, e))?
    {
        let call_entry = call_entry.map_err(|e| WorkflowError::io(branches_root, e))?;
        let call_path = call_entry.path();
        if !call_path.is_dir() {
            continue;
        }
        let call_id = call_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_string();
        for entry in fs::read_dir(&call_path).map_err(|e| WorkflowError::io(&call_path, e))? {
            let entry = entry.map_err(|e| WorkflowError::io(&call_path, e))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let raw = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
            let value: serde_json::Value = serde_json::from_str(&raw)?;
            if contains_forbidden_field(&value) {
                continue;
            }
            let clean = sanitize_value(value);
            let result = clean.get("result").unwrap_or(&serde_json::Value::Null);
            out.push(WorkflowV2BranchView {
                call_id: call_id.clone(),
                item_id: string_field(&clean, "item_id").unwrap_or_else(|| "unknown".to_string()),
                role: string_field(&clean, "role").unwrap_or_else(|| "agent".to_string()),
                status: string_field(&clean, "status").unwrap_or_else(|| "unknown".to_string()),
                summary: string_field(result, "summary"),
                error: string_field(&clean, "error"),
                output_path: path
                    .strip_prefix(root)
                    .unwrap_or(path.as_path())
                    .display()
                    .to_string(),
            });
        }
    }
    Ok(())
}

fn branch_count_for_call(root: &Path, call_id: &str) -> usize {
    let branch_root = root.join("branches").join(sanitize_v2_id(call_id));
    fs::read_dir(branch_root)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(|entry| entry.ok()))
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .count()
}

fn sanitize_v2_id(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn collect_agent_views(
    root: &Path,
    dir: &Path,
    prompt_index: &PromptIndex,
    out: &mut Vec<WorkflowAgentView>,
) -> WorkflowResult<()> {
    for entry in fs::read_dir(dir).map_err(|e| WorkflowError::io(dir, e))? {
        let entry = entry.map_err(|e| WorkflowError::io(dir, e))?;
        let path = entry.path();
        if path.is_dir() {
            collect_agent_views(root, &path, prompt_index, out)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        if contains_forbidden_field(&value) {
            continue;
        }
        let clean = sanitize_value(value);
        out.push(agent_view(root, &path, prompt_index, &clean));
    }
    Ok(())
}

fn agent_view(
    root: &Path,
    path: &Path,
    prompt_index: &PromptIndex,
    value: &serde_json::Value,
) -> WorkflowAgentView {
    let artifact = value.get("artifact");
    let stage_id = string_field(value, "stage_id").unwrap_or_default();
    let item_id = string_field(value, "item_id").unwrap_or_default();
    let prompt = prompt_index.get(&(stage_id.clone(), item_id.clone()));
    WorkflowAgentView {
        stage_id,
        item_id,
        status: string_field(value, "status").unwrap_or_else(|| "unknown".to_string()),
        prompt_path: prompt.map(|prompt| prompt.path.clone()),
        input_hash: prompt.and_then(|prompt| prompt.input_hash.clone()),
        prompt_hash: prompt.and_then(|prompt| prompt.prompt_hash.clone()),
        prompt_created_at: prompt.and_then(|prompt| prompt.created_at.clone()),
        provider: string_field(value, "provider"),
        model: string_field(value, "model"),
        tokens_in: value
            .get("tokens_in")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        tokens_out: value
            .get("tokens_out")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0),
        cost_usd: value
            .get("cost_usd")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        artifact_id: artifact.and_then(|a| string_field(a, "id")),
        artifact_path: artifact.and_then(|a| string_field(a, "path")),
        recent_public_tool_calls: value
            .get("recent_public_tool_calls")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default(),
        result_preview: value.get("body").map(preview_value),
        error: string_field(value, "error"),
        output_path: path
            .strip_prefix(root)
            .unwrap_or(path)
            .display()
            .to_string(),
    }
}

#[derive(Debug, Clone, Default)]
struct PromptMeta {
    path: String,
    input_hash: Option<String>,
    prompt_hash: Option<String>,
    created_at: Option<String>,
}

type PromptIndex = BTreeMap<(String, String), PromptMeta>;

fn prompt_index(root: &Path) -> WorkflowResult<PromptIndex> {
    let mut index = PromptIndex::new();
    if root.exists() {
        collect_prompt_index(root, root, &mut index)?;
    }
    Ok(index)
}

fn collect_prompt_index(root: &Path, dir: &Path, index: &mut PromptIndex) -> WorkflowResult<()> {
    for entry in fs::read_dir(dir).map_err(|e| WorkflowError::io(dir, e))? {
        let entry = entry.map_err(|e| WorkflowError::io(dir, e))?;
        let path = entry.path();
        if path.is_dir() {
            collect_prompt_index(root, &path, index)?;
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
        let value: serde_json::Value = serde_json::from_str(&raw)?;
        if contains_forbidden_field(&value) {
            continue;
        }
        let clean = sanitize_value(value);
        let Some(stage_id) = string_field(&clean, "stage_id") else {
            continue;
        };
        let Some(item_id) = string_field(&clean, "item_id") else {
            continue;
        };
        index.insert(
            (stage_id, item_id),
            PromptMeta {
                path: path
                    .strip_prefix(root)
                    .unwrap_or(path.as_path())
                    .display()
                    .to_string(),
                input_hash: string_field(&clean, "input_hash"),
                prompt_hash: string_field(&clean, "prompt_hash"),
                created_at: string_field(&clean, "created_at"),
            },
        );
    }
    Ok(())
}

fn string_field(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn preview_value(value: &serde_json::Value) -> String {
    let raw = match value {
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    const MAX: usize = 800;
    if raw.len() <= MAX {
        raw
    } else {
        let preview = raw.chars().take(MAX).collect::<String>();
        format!("{preview}...")
    }
}

fn stage_views(run: &WorkflowRun) -> Vec<WorkflowStageView> {
    run.stages
        .values()
        .map(|stage| WorkflowStageView {
            id: stage.id.clone(),
            status: stage.status,
            attempt: stage.attempt,
            started_at: stage.started_at.map(|ts| ts.to_rfc3339()),
            completed_at: stage.completed_at.map(|ts| ts.to_rfc3339()),
            artifacts: stage.artifacts.len(),
            error: stage.error.clone(),
        })
        .collect()
}

fn is_tool_noise(event: &WorkflowEvent) -> bool {
    event
        .detail
        .get("raw_tool_output")
        .or_else(|| event.detail.get("tool_stdout"))
        .is_some()
}

fn event_summary(event: &WorkflowEvent) -> String {
    event
        .detail
        .get("stage")
        .or_else(|| event.detail.get("name"))
        .or_else(|| event.detail.get("status"))
        .or_else(|| event.detail.get("error_class"))
        .and_then(|value| value.as_str())
        .unwrap_or("workflow event")
        .to_string()
}

fn status_label(kind: &WorkflowEventKind) -> &'static str {
    match kind {
        WorkflowEventKind::Started | WorkflowEventKind::StageStarted => "running",
        WorkflowEventKind::StageCompleted | WorkflowEventKind::Completed => "completed",
        WorkflowEventKind::StageStalled => "stalled",
        WorkflowEventKind::StageFailed => "failed",
        WorkflowEventKind::StageSkipped => "skipped",
        WorkflowEventKind::ForcedAccepted => "forced",
        WorkflowEventKind::Resumed => "resumed",
        WorkflowEventKind::Paused => "paused",
        WorkflowEventKind::Cancelled => "cancelled",
        WorkflowEventKind::LearningRecorded => "learning",
        _ => "write_coordination",
    }
}

fn control_previews() -> Vec<WorkflowControlPreview> {
    [
        "approve-run-once",
        "approve-always",
        "deny-workflow",
        "resume",
        "continue",
        "repair",
        "pause",
        "cancel",
        "restart-stage",
        "restart-item",
    ]
    .into_iter()
    .map(|action| WorkflowControlPreview {
        action: action.to_string(),
        enabled: true,
        policy_reason: "policy gate checked server-side before mutation".to_string(),
    })
    .collect()
}

impl From<&WorkflowRun> for WorkflowRunSummary {
    fn from(run: &WorkflowRun) -> Self {
        let accepted_count = run
            .stages
            .values()
            .filter(|stage| run.accepted_stage(&stage.id))
            .count();
        let failed_count = run
            .stages
            .values()
            .filter(|stage| matches!(stage.status, StageStatus::Failed))
            .count();
        Self {
            id: run.id.clone(),
            name: run.spec.name.clone(),
            status: run.status.clone(),
            stage_count: run.stages.len(),
            accepted_count,
            failed_count,
            artifact_count: artifact_views(run).len(),
            updated_at: run.updated_at.to_rfc3339(),
        }
    }
}
