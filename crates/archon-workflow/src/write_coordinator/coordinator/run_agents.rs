use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::Semaphore;

use super::{FanoutCtx, ItemState};
use crate::control::{RunControl, RunControlDecision};
use crate::persistence;
use crate::request::IMPLEMENTATION_CONSTRAINTS;
use crate::request::fanout_item_request;
use crate::runner::{StageRunOutput, StageRunRequest, WorkflowStageRunner};
use crate::write_coordinator::ItemId;
use crate::write_coordinator::worktree_isolation::detect_canonical_mutation;
use crate::write_coordinator::write_plan::NormalizedPath;

const MAX_EVIDENCE_BYTES: u64 = 128 * 1024;
const MAX_EVIDENCE_FILES: usize = 32;

pub(super) type ItemRunOutputs = BTreeMap<ItemId, StageRunOutput>;

pub(super) async fn run_wave_agents(
    ctx: &FanoutCtx<'_>,
    canonical: &Path,
    runner: &dyn WorkflowStageRunner,
    items: &[ItemState<'_>],
    width: usize,
) -> Result<ItemRunOutputs, String> {
    let sema = Arc::new(Semaphore::new(width.max(1)));
    let futures = items.iter().map(|it| {
        let sema = sema.clone();
        async move {
            let _permit = sema.acquire().await.expect("semaphore open");
            match RunControl::new(ctx.store.clone(), ctx.run.id.clone()).poll() {
                Ok(RunControlDecision::Continue) => {}
                Ok(RunControlDecision::Paused { generation }) => {
                    return Err(format!("ControlPaused: generation {generation}"));
                }
                Ok(RunControlDecision::Cancelled { generation }) => {
                    return Err(format!("ControlCancelled: generation {generation}"));
                }
                Err(err) => {
                    return Err(format!("ControlPollFailed: {err}"));
                }
            }
            run_one_agent(ctx, canonical, runner, it).await
        }
    });
    let mut bodies = BTreeMap::new();
    for result in futures_util::future::join_all(futures).await {
        let (item_id, body) = result?;
        bodies.insert(item_id, body);
    }
    Ok(bodies)
}

async fn run_one_agent(
    ctx: &FanoutCtx<'_>,
    canonical: &Path,
    runner: &dyn WorkflowStageRunner,
    it: &ItemState<'_>,
) -> Result<(ItemId, StageRunOutput), String> {
    let mut req = build_item_request(ctx, canonical, it);
    let mut output = run_agent_request(ctx, runner, it, &req).await?;
    if output_asks_for_confirmation(&output.body) {
        req = retry_request(&req, &output.body);
        output = run_agent_request(ctx, runner, it, &req).await?;
    }
    detect_canonical_mutation(
        canonical,
        &it.baseline,
        &it.plan.target_files,
        &ctx.verify_inputs,
    )
    .map_err(|err| format!("CanonicalMutation: {} ({err})", it.plan.item_id))?;
    Ok((it.plan.item_id.clone(), output))
}

async fn run_agent_request(
    ctx: &FanoutCtx<'_>,
    runner: &dyn WorkflowStageRunner,
    it: &ItemState<'_>,
    req: &StageRunRequest,
) -> Result<StageRunOutput, String> {
    persistence::record_prompt(ctx.store, req)
        .map_err(|err| format!("RecordPrompt: {} ({err})", it.plan.item_id))?;
    runner
        .run_stage(req.clone())
        .await
        .map_err(|err| format!("AgentFailed: {} ({err})", it.plan.item_id))
}

fn retry_request(request: &StageRunRequest, previous_body: &str) -> StageRunRequest {
    let mut retry = request.clone();
    retry.attempt = retry.attempt.saturating_add(1);
    retry.task = format!(
        "{}\n\nWorkflow corrective retry: the previous response asked for confirmation or returned a plan-only answer. Do not ask whether to proceed. Execute the stage now using the available tools. Modify the declared target files directly under `target_repository_root`, or return the exact idempotent_noop JSON only if no repository change is required.",
        request.task
    );
    if let Some(obj) = retry.input.as_object_mut() {
        obj.insert(
            "workflow_retry".into(),
            json!({
                "reason": "previous_output_asked_for_confirmation",
                "previous_output_excerpt": one_line_excerpt(previous_body, 400),
            }),
        );
    }
    retry
}

fn output_asks_for_confirmation(body: &str) -> bool {
    let lower = body.to_ascii_lowercase();
    [
        "would you like me to proceed",
        "do you want me to proceed",
        "should i proceed",
        "shall i proceed",
        "would you like me to continue",
        "do you want me to continue",
        "let me know if you want me to proceed",
        "let me know if you'd like me to proceed",
        "if you want me to proceed",
    ]
    .iter()
    .any(|phrase| lower.contains(phrase))
}

fn one_line_excerpt(text: &str, max: usize) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(max).collect()
}

fn build_item_request(
    ctx: &FanoutCtx<'_>,
    canonical: &Path,
    it: &ItemState<'_>,
) -> crate::runner::StageRunRequest {
    let mut req = fanout_item_request(ctx.store, ctx.run, ctx.stage, &it.input.item)
        .unwrap_or_else(|_| panic!("fanout_item_request for {}", it.plan.item_id));
    if !req.input.is_object() {
        req.input = json!({});
    }
    let declared: Vec<String> = it
        .plan
        .target_files
        .iter()
        .map(NormalizedPath::as_str)
        .collect();
    let obj = req.input.as_object_mut().expect("input is object");
    obj.insert(
        "target_repository_root".into(),
        json!(it.plan.isolated_root.to_string_lossy()),
    );
    obj.insert(
        "write_coordination".into(),
        json!({
            "enabled": true,
            "canonical_repository_root": canonical.to_string_lossy(),
            "declared_target_files": declared,
            "hard_engineering_constraints": IMPLEMENTATION_CONSTRAINTS,
        }),
    );
    obj.insert(
        "hard_engineering_constraints".into(),
        json!(IMPLEMENTATION_CONSTRAINTS),
    );
    attach_task_evidence(obj);
    req
}

fn attach_task_evidence(obj: &mut serde_json::Map<String, Value>) {
    let candidates = task_evidence_candidates(obj);
    let mut evidence = evidence_from_sources(obj, &candidates);
    for candidate in &candidates {
        if evidence.len() >= MAX_EVIDENCE_FILES {
            break;
        }
        if !evidence_matches(&evidence, candidate)
            && let Some(file) = read_candidate(candidate)
        {
            evidence.push(file);
        }
    }
    if !evidence.is_empty() {
        obj.insert("task_evidence".into(), json!(evidence));
    }
}

fn task_evidence_candidates(obj: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    if let Some(item) = obj.get("fanout_item") {
        collect_key_paths(item, "task_file", &mut out, &mut seen);
        collect_key_paths(item, "task_files", &mut out, &mut seen);
        collect_markdown_paths(item.get("task"), &mut out, &mut seen);
        collect_markdown_paths(item.get("source_files"), &mut out, &mut seen);
    }
    collect_markdown_paths(obj.get("workflow_task"), &mut out, &mut seen);
    collect_markdown_paths(obj.get("stage_task"), &mut out, &mut seen);
    collect_markdown_paths(obj.get("context"), &mut out, &mut seen);
    expand_task_pack_context(&mut out, &mut seen);
    out
}

fn collect_key_paths(value: &Value, key: &str, out: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    match value.get(key) {
        Some(Value::String(path)) if !path.trim().is_empty() => {
            push_candidate(out, seen, path);
        }
        Some(Value::Array(paths)) => paths.iter().filter_map(Value::as_str).for_each(|path| {
            if !path.trim().is_empty() {
                push_candidate(out, seen, path);
            }
        }),
        _ => {}
    }
}

fn collect_markdown_paths(
    value: Option<&Value>,
    out: &mut Vec<String>,
    seen: &mut BTreeSet<String>,
) {
    match value {
        Some(Value::String(text)) => text
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || "/._-~".contains(ch)))
            .filter(|part| part.starts_with('/') && part.ends_with(".md"))
            .for_each(|path| {
                push_candidate(out, seen, path.trim_matches(['`', '.', ',']));
            }),
        Some(Value::Array(items)) => items
            .iter()
            .for_each(|item| collect_markdown_paths(Some(item), out, seen)),
        Some(Value::Object(map)) => map
            .values()
            .for_each(|item| collect_markdown_paths(Some(item), out, seen)),
        _ => {}
    }
}

fn push_candidate(out: &mut Vec<String>, seen: &mut BTreeSet<String>, path: &str) {
    let path = path.trim();
    if !path.is_empty() && seen.insert(path.to_string()) {
        out.push(path.to_string());
    }
}

fn expand_task_pack_context(out: &mut Vec<String>, seen: &mut BTreeSet<String>) {
    let task_files: Vec<String> = out
        .iter()
        .filter(|path| path.contains("/tasks/") && path.ends_with(".md"))
        .cloned()
        .collect();
    for task_file in task_files {
        let Some(root) = task_pack_root(Path::new(&task_file)) else {
            continue;
        };
        add_prd_candidates(out, seen, &root);
        add_if_file(out, seen, &root.join("README.md"));
        add_markdown_dir(out, seen, &root.join("specs"));
        add_markdown_dir(out, seen, &root.join("context"));
        add_task_markdown(out, seen, &root);
    }
}

fn task_pack_root(path: &Path) -> Option<std::path::PathBuf> {
    let mut dir = path.parent()?;
    loop {
        if dir.join("README.md").is_file() && contains_task_markdown(dir) {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

fn contains_task_markdown(dir: &Path) -> bool {
    fs::read_dir(dir).ok().into_iter().flatten().any(|entry| {
        entry.ok().is_some_and(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("TASK-") && name.ends_with(".md"))
        })
    })
}

fn add_markdown_dir(out: &mut Vec<String>, seen: &mut BTreeSet<String>, dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            add_if_file(out, seen, &path);
        }
    }
}

fn add_task_markdown(out: &mut Vec<String>, seen: &mut BTreeSet<String>, dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("TASK-") && name.ends_with(".md"))
        {
            add_if_file(out, seen, &path);
        }
    }
}

fn add_prd_candidates(out: &mut Vec<String>, seen: &mut BTreeSet<String>, task_root: &Path) {
    let Some(project_root) = task_root.parent().and_then(Path::parent) else {
        return;
    };
    let Some(pack_name) = task_root.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    let prds = project_root.join("prds");
    for path in [
        prds.join(format!("{pack_name}.md")),
        prds.join(pack_name).join("PRD.md"),
    ] {
        add_if_file(out, seen, &path);
    }
}

fn add_if_file(out: &mut Vec<String>, seen: &mut BTreeSet<String>, path: &Path) {
    if path.is_file() {
        push_candidate(out, seen, &path.display().to_string());
    }
}

fn evidence_from_sources(
    obj: &serde_json::Map<String, Value>,
    candidates: &[String],
) -> Vec<Value> {
    obj.get("source_files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|source| is_task_source(source, candidates))
        .take(MAX_EVIDENCE_FILES)
        .cloned()
        .collect()
}

fn is_task_source(source: &Value, candidates: &[String]) -> bool {
    let Some(path) = source_path(source) else {
        return false;
    };
    candidates
        .iter()
        .any(|candidate| paths_match(&path, candidate))
        || (path.contains("/tasks/") && path.ends_with(".md"))
}

fn evidence_matches(evidence: &[Value], candidate: &str) -> bool {
    evidence
        .iter()
        .filter_map(source_path)
        .any(|path| paths_match(&path, candidate))
}

fn paths_match(path: &str, candidate: &str) -> bool {
    path == candidate || path.ends_with(candidate) || candidate.ends_with(path)
}

fn source_path(source: &Value) -> Option<String> {
    source
        .get("absolute_path")
        .or_else(|| source.get("path"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn read_candidate(path: &str) -> Option<Value> {
    let path = Path::new(path);
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let truncated = metadata.len() > MAX_EVIDENCE_BYTES;
    let content = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_EVIDENCE_BYTES as usize)]);
    Some(json!({
        "path": path.display().to_string(),
        "absolute_path": path.display().to_string(),
        "exists": true,
        "bytes": metadata.len(),
        "truncated": truncated,
        "content": content,
    }))
}
