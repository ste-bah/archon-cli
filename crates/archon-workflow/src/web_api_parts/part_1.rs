use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::approval::{
    WorkflowApprovalDecision, WorkflowApprovalInspection, WorkflowApprovalStore,
    project_root_from_workflow_root,
};
use crate::bundle::{
    COMPILED_SPEC_FILE, HARNESS_FILE, WorkflowBundle, WorkflowBundleOrigin, read_manifest,
};
use crate::error::{WorkflowError, WorkflowResult};
use crate::events::{WorkflowEvent, WorkflowEventKind, contains_forbidden_field, sanitize_value};
use crate::run::{RunStatus, StageStatus, WorkflowRun};
use crate::store::WorkflowStore;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowWebSummary {
    pub root: String,
    pub runs: Vec<WorkflowRunSummary>,
    pub events: Vec<WorkflowEventPreview>,
    pub controls: Vec<WorkflowControlPreview>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunSummary {
    pub id: String,
    pub name: String,
    pub status: RunStatus,
    pub stage_count: usize,
    pub accepted_count: usize,
    pub failed_count: usize,
    pub artifact_count: usize,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowRunDetail {
    pub summary: WorkflowRunSummary,
    pub bundle: Option<WorkflowBundleView>,
    pub approval: Option<WorkflowApprovalView>,
    pub harness: Option<String>,
    pub compiled_spec: Option<String>,
    pub stages: Vec<WorkflowStageView>,
    pub agents: Vec<WorkflowAgentView>,
    pub v2_results: Vec<WorkflowV2ResultView>,
    pub v2_branches: Vec<WorkflowV2BranchView>,
    pub artifacts: Vec<WorkflowArtifactView>,
    pub events: Vec<WorkflowEventPreview>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowStageView {
    pub id: String,
    pub status: StageStatus,
    pub attempt: u32,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub artifacts: usize,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowArtifactView {
    pub id: String,
    pub path: String,
    pub producing_stage: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowBundleView {
    pub workflow_path: String,
    pub compiled_spec_path: String,
    pub workflow_hash: String,
    pub compiled_hash: String,
    pub phase_count: usize,
    pub max_agents: u32,
    pub max_parallelism: u32,
    pub write_capable_stages: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowApprovalView {
    pub workflow_hash: String,
    pub compiled_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated_metadata_hash: Option<String>,
    pub approval_subject_hash: String,
    pub project_root: String,
    pub workflow_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<WorkflowBundleOrigin>,
    pub phase_count: usize,
    pub max_agents: u32,
    pub max_parallelism: u32,
    pub write_capable_stages: Vec<String>,
    pub external_requirements: Vec<String>,
    pub cost_warning: String,
    pub raw_script_path: String,
    pub compiled_spec_path: String,
    pub decision: Option<String>,
    pub decided_at: Option<String>,
    pub decided_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowAgentView {
    pub stage_id: String,
    pub item_id: String,
    pub status: String,
    pub prompt_path: Option<String>,
    pub input_hash: Option<String>,
    pub prompt_hash: Option<String>,
    pub prompt_created_at: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub artifact_id: Option<String>,
    pub artifact_path: Option<String>,
    pub recent_public_tool_calls: Vec<serde_json::Value>,
    pub result_preview: Option<String>,
    pub error: Option<String>,
    pub output_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowV2ResultView {
    pub call_id: String,
    pub status: String,
    pub summary: String,
    pub result_path: String,
    pub artifact_count: usize,
    pub branch_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowV2BranchView {
    pub call_id: String,
    pub item_id: String,
    pub role: String,
    pub status: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub output_path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowEventPreview {
    pub run_id: String,
    pub seq: u64,
    pub kind: WorkflowEventKind,
    pub status: String,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowControlPreview {
    pub action: String,
    pub enabled: bool,
    pub policy_reason: String,
}

pub fn summary(store: &WorkflowStore, limit: usize) -> WorkflowResult<WorkflowWebSummary> {
    let runs = store
        .list_runs()?
        .into_iter()
        .take(limit)
        .collect::<Vec<_>>();
    let events = runs
        .iter()
        .flat_map(|run| event_previews(store, &run.id, 8).unwrap_or_default())
        .take(limit)
        .collect();
    Ok(WorkflowWebSummary {
        root: store.root().display().to_string(),
        runs: runs.iter().map(WorkflowRunSummary::from).collect(),
        events,
        controls: control_previews(),
    })
}

pub fn detail(store: &WorkflowStore, run_id: &str) -> WorkflowResult<WorkflowRunDetail> {
    let run = store.load_state(run_id)?;
    let bundle = bundle_view(store, run_id).ok();
    let approval = approval_view(store, &run).ok();
    Ok(WorkflowRunDetail {
        summary: WorkflowRunSummary::from(&run),
        harness: read_harness(store, run_id).ok(),
        compiled_spec: read_compiled_spec(store, run_id).ok(),
        bundle,
        approval,
        stages: stage_views(&run),
        agents: agent_views(store, run_id)?,
        v2_results: v2_result_views(store, run_id)?,
        v2_branches: v2_branch_views(store, run_id)?,
        artifacts: artifact_views(&run),
        events: event_previews(store, run_id, 200)?,
    })
}

pub fn event_previews(
    store: &WorkflowStore,
    run_id: &str,
    limit: usize,
) -> WorkflowResult<Vec<WorkflowEventPreview>> {
    let mut events = read_events(store, run_id)?
        .into_iter()
        .filter(|event| !is_tool_noise(event))
        .map(|event| {
            let status = status_label(&event.kind).to_string();
            let summary = event_summary(&event);
            WorkflowEventPreview {
                run_id: event.run_id,
                seq: event.seq,
                kind: event.kind,
                status,
                summary,
                created_at: event.ts.to_rfc3339(),
            }
        })
        .collect::<Vec<_>>();
    events.sort_by_key(|event| std::cmp::Reverse(event.seq));
    events.truncate(limit);
    Ok(events)
}

pub fn event_previews_after(
    store: &WorkflowStore,
    run_id: &str,
    after_seq: u64,
    limit: usize,
) -> WorkflowResult<Vec<WorkflowEventPreview>> {
    let mut events = read_events(store, run_id)?
        .into_iter()
        .filter(|event| event.seq > after_seq)
        .filter(|event| !is_tool_noise(event))
        .map(|event| {
            let status = status_label(&event.kind).to_string();
            let summary = event_summary(&event);
            WorkflowEventPreview {
                run_id: event.run_id,
                seq: event.seq,
                kind: event.kind,
                status,
                summary,
                created_at: event.ts.to_rfc3339(),
            }
        })
        .collect::<Vec<_>>();
    events.sort_by_key(|event| event.seq);
    events.truncate(limit);
    Ok(events)
}

pub fn artifact_views(run: &WorkflowRun) -> Vec<WorkflowArtifactView> {
    run.stages
        .values()
        .flat_map(|stage| stage.artifacts.iter())
        .map(|artifact| WorkflowArtifactView {
            id: artifact.id.clone(),
            path: artifact.path.display().to_string(),
            producing_stage: artifact.producing_stage.clone(),
            content_hash: artifact.content_hash.clone(),
        })
        .collect()
}

fn read_events(store: &WorkflowStore, run_id: &str) -> WorkflowResult<Vec<WorkflowEvent>> {
    let path = store.events_path(run_id);
    let raw = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
    let mut events = Vec::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let event: WorkflowEvent = serde_json::from_str(line)?;
        if !contains_forbidden_field(&event.detail) {
            events.push(event);
        }
    }
    Ok(events)
}

fn read_harness(store: &WorkflowStore, run_id: &str) -> WorkflowResult<String> {
    let path = store.run_dir(run_id).join(HARNESS_FILE);
    let raw = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
    let value = sanitize_value(serde_json::json!({ "source": raw }));
    Ok(value
        .get("source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string())
}

fn read_compiled_spec(store: &WorkflowStore, run_id: &str) -> WorkflowResult<String> {
    let path = store.run_dir(run_id).join(COMPILED_SPEC_FILE);
    let raw = fs::read_to_string(&path).map_err(|e| WorkflowError::io(&path, e))?;
    let value = sanitize_value(serde_json::json!({ "source": raw }));
    Ok(value
        .get("source")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string())
}

fn bundle_view(store: &WorkflowStore, run_id: &str) -> WorkflowResult<WorkflowBundleView> {
    WorkflowBundle::verify(store, run_id)?;
    let manifest = read_manifest(store, run_id)?;
    Ok(WorkflowBundleView {
        workflow_path: HARNESS_FILE.to_string(),
        compiled_spec_path: COMPILED_SPEC_FILE.to_string(),
        workflow_hash: manifest.workflow_hash,
        compiled_hash: manifest.compiled_hash,
        phase_count: manifest.phase_count,
        max_agents: manifest.max_agents,
        max_parallelism: manifest.max_parallelism,
        write_capable_stages: manifest.write_capable_stages,
    })
}

fn approval_view(store: &WorkflowStore, run: &WorkflowRun) -> WorkflowResult<WorkflowApprovalView> {
    let project_root = project_root_from_workflow_root(store.root());
    let approvals = WorkflowApprovalStore::project(&project_root);
    approvals
        .inspect_run(&project_root, store, run)
        .map(WorkflowApprovalView::from)
}

impl From<WorkflowApprovalInspection> for WorkflowApprovalView {
    fn from(value: WorkflowApprovalInspection) -> Self {
        let (decision, decided_at, decided_by) = value
            .decision
            .map(|record| {
                (
                    Some(approval_decision_label(&record.decision).to_string()),
                    Some(record.decided_at),
                    Some(record.decided_by),
                )
            })
            .unwrap_or((None, None, None));
        Self {
            workflow_hash: value.workflow_hash,
            compiled_hash: value.compiled_hash,
            generated_metadata_hash: value.generated_metadata_hash,
            approval_subject_hash: value.approval_subject_hash,
            project_root: value.project_root,
            workflow_name: value.workflow_name,
            origin: value.origin,
            phase_count: value.phase_count,
            max_agents: value.max_agents,
            max_parallelism: value.max_parallelism,
            write_capable_stages: value.write_capable_stages,
            external_requirements: value.external_requirements,
            cost_warning: value.cost_warning,
            raw_script_path: value.raw_script_path,
            compiled_spec_path: value.compiled_spec_path,
            decision,
            decided_at,
            decided_by,
        }
    }
}

fn approval_decision_label(decision: &WorkflowApprovalDecision) -> &'static str {
    match decision {
        WorkflowApprovalDecision::RunOnce => "run_once",
        WorkflowApprovalDecision::AlwaysForProject => "always_for_project",
        WorkflowApprovalDecision::Denied => "denied",
    }
}

fn agent_views(store: &WorkflowStore, run_id: &str) -> WorkflowResult<Vec<WorkflowAgentView>> {
    let root = store.run_dir(run_id).join("agent-outputs");
    if !root.exists() {
        return Ok(Vec::new());
    }
    let prompt_index = prompt_index(&store.run_dir(run_id).join("prompts")).unwrap_or_default();
    let mut out = Vec::new();
    collect_agent_views(&root, &root, &prompt_index, &mut out)?;
    out.sort_by(|a, b| {
        a.stage_id
            .cmp(&b.stage_id)
            .then_with(|| a.item_id.cmp(&b.item_id))
    });
    Ok(out)
}

