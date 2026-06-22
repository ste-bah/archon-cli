use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use archon_pipeline::runner::LlmClient;
use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;
use archon_workflow::{
    ProviderTier, WorkflowError, WorkflowSpec, WorkflowStore, WorkflowV2HarnessValidator,
    WorkflowV2HostCall,
};

use super::workflow_live_compat::compatibility_spec_from_v2_calls;
use super::workflow_live_prompt::{harness_planner_prompt, harness_repair_prompt};
use super::workflow_live_retry;
use super::workflow_live_runner::tier_model_alias;

#[derive(Debug, Clone)]
pub(super) struct LivePlan {
    pub(super) spec: WorkflowSpec,
    pub(super) harness_source: String,
    pub(super) calls: Vec<WorkflowV2HostCall>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct PlannerFailureAttempt {
    kind: &'static str,
    attempt: usize,
    error: Option<String>,
    content_hash: String,
    content: String,
    content_preview: String,
}

#[derive(Debug, Clone)]
struct PlannerFailure {
    error: String,
    attempts: Vec<PlannerFailureAttempt>,
}

impl PlannerFailure {
    fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            attempts: Vec::new(),
        }
    }

    fn with_attempts(error: impl Into<String>, attempts: Vec<PlannerFailureAttempt>) -> Self {
        Self {
            error: error.into(),
            attempts,
        }
    }
}

pub(super) async fn plan_live(
    store: &WorkflowStore,
    task: &str,
    llm: Arc<dyn LlmClient>,
    tui_tx: TuiEventSender,
) -> Result<LivePlan> {
    let _ = tui_tx.send(TuiEvent::TextDelta(
        "Workflow planner: requesting workflow.js harness from active provider; no run directory exists until validation passes.\n"
            .into(),
    ));
    match llm_plan(task, llm, &tui_tx).await {
        Ok(plan) => {
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "Workflow planner: validated V2 harness '{}' with {} host call(s); creating run...\n",
                plan.spec.name,
                plan.calls.len()
            )));
            Ok(plan)
        }
        Err(failure) => {
            let message = planner_failure_message(store, task, &failure.error, &failure.attempts);
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "Workflow planner failed workflow.js safety validation; live mode will not fall back to a fixed pipeline: {message}\n"
            )));
            Err(anyhow!(message))
        }
    }
}

fn planner_failure_message(
    store: &WorkflowStore,
    task: &str,
    error: &str,
    attempts: &[PlannerFailureAttempt],
) -> String {
    match record_planner_failure(store, task, error, attempts) {
        Ok(path) => format!("{error}; planner failure recorded at {}", path.display()),
        Err(log_err) => format!("{error}; planner failure recording also failed: {log_err}"),
    }
}

fn record_planner_failure(
    store: &WorkflowStore,
    task: &str,
    error: &str,
    attempts: &[PlannerFailureAttempt],
) -> Result<PathBuf> {
    let dir = store.root().join("planner-failures");
    fs::create_dir_all(&dir)?;
    let created_at = chrono::Utc::now().to_rfc3339();
    let id = chrono::Utc::now().timestamp_millis();
    let target = dir.join(format!("planner-failure-{id}.json"));
    let tmp = target.with_extension("json.tmp");
    let body = serde_json::json!({
        "schema": "archon.workflow.planner_failure.v1",
        "created_at": created_at,
        "task": task,
        "error": error,
        "attempts": attempts,
    });
    fs::write(&tmp, serde_json::to_vec_pretty(&body)?)?;
    fs::rename(&tmp, &target)?;
    Ok(target)
}

async fn llm_plan(
    task: &str,
    llm: Arc<dyn LlmClient>,
    tui_tx: &TuiEventSender,
) -> std::result::Result<LivePlan, PlannerFailure> {
    let response = workflow_live_retry::send_message_with_transient_retry(
        &llm,
        vec![serde_json::json!({
            "role": "user",
            "content": harness_planner_prompt(task),
        })],
        vec![serde_json::json!({
            "type": "text",
            "text": "You are Archon's provider-neutral dynamic workflow planner. Return only workflow.js JavaScript using the allowed w.* host API. Do not include hidden reasoning, credentials, provider names, model names, imports, filesystem, network, shell, or eval.",
        })],
        Vec::new(),
        tier_model_alias(ProviderTier::Planner),
        |attempt| {
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "Workflow planner: transient provider error; retrying harness request ({attempt}/3)...\n"
            )));
        },
    )
    .await
    .map_err(|err| PlannerFailure::new(err.to_string()))?;
    let raw = extract_javascript(&response.content);
    validate_or_repair_harness(task, raw, llm, tui_tx).await
}

async fn validate_or_repair_harness(
    task: &str,
    raw: String,
    llm: Arc<dyn LlmClient>,
    tui_tx: &TuiEventSender,
) -> std::result::Result<LivePlan, PlannerFailure> {
    const MAX_REPAIRS: usize = 2;
    let mut harness = raw;
    let mut attempts = Vec::new();
    for attempt in 0..=MAX_REPAIRS {
        match compile_harness_plan(task, &harness) {
            Ok(plan) => return Ok(plan),
            Err(err) if attempt < MAX_REPAIRS => {
                let error = err.to_string();
                attempts.push(planner_attempt(
                    "harness",
                    attempt + 1,
                    Some(&error),
                    &harness,
                ));
                let repair_number = attempt + 1;
                let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                    "Workflow planner: generated harness failed validation ({err}); requesting repaired harness ({repair_number}/{MAX_REPAIRS})...\n"
                )));
                harness = request_repaired_harness(task, &harness, error, llm.clone())
                    .await
                    .map_err(|err| {
                        PlannerFailure::with_attempts(err.to_string(), attempts.clone())
                    })?;
            }
            Err(err) => {
                let error = err.to_string();
                attempts.push(planner_attempt(
                    "harness",
                    attempt + 1,
                    Some(&error),
                    &harness,
                ));
                return Err(PlannerFailure::with_attempts(error, attempts));
            }
        }
    }
    unreachable!("harness repair loop either returns plan or final error")
}

fn planner_attempt(
    kind: &'static str,
    attempt: usize,
    error: Option<&str>,
    content: &str,
) -> PlannerFailureAttempt {
    PlannerFailureAttempt {
        kind,
        attempt,
        error: error.map(str::to_string),
        content_hash: {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(content.as_bytes()))
        },
        content: content.to_string(),
        content_preview: truncate_chars(content.trim(), 4000),
    }
}

fn truncate_chars(value: &str, max: usize) -> String {
    let mut out = value.chars().take(max).collect::<String>();
    if value.chars().count() > max {
        out.push_str("\n...[truncated]");
    }
    out
}

fn compile_harness_plan(
    task: &str,
    harness_source: &str,
) -> archon_workflow::WorkflowResult<LivePlan> {
    let plan = WorkflowV2HarnessValidator::default()
        .validate(harness_source)
        .map_err(|err| WorkflowError::SpecInvalid(err.to_string()))?;
    let spec = compatibility_spec_from_v2_calls(task, &plan.calls);
    Ok(LivePlan {
        spec,
        harness_source: harness_source.trim().to_string(),
        calls: plan.calls,
    })
}

async fn request_repaired_harness(
    task: &str,
    invalid_harness: &str,
    error: String,
    llm: Arc<dyn LlmClient>,
) -> Result<String> {
    let response = workflow_live_retry::send_message_with_transient_retry(
        &llm,
        vec![serde_json::json!({
            "role": "user",
            "content": harness_repair_prompt(task, invalid_harness, &error),
        })],
        vec![serde_json::json!({
            "type": "text",
            "text": "Repair the workflow.js harness only. Use only the allowed w.* host API and preserve provider neutrality.",
        })],
        Vec::new(),
        tier_model_alias(ProviderTier::Planner),
        |_| {},
    )
    .await?;
    Ok(extract_javascript(&response.content))
}

pub(super) fn render_live_plan(plan: &LivePlan) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!(
        "Workflow V2 harness validated: {} ({} host call(s))\n",
        plan.spec.name,
        plan.calls.len()
    ));
    for call in &plan.calls {
        out.push_str(&format!(
            "- {}: w.{} write_mode={:?}\n",
            call.id,
            call.method.as_str(),
            call.write_mode
        ));
    }
    out.push_str("\nworkflow.js:\n");
    out.push_str(&plan.harness_source);
    out.push_str("\n\nworkflow.v2.compat.yaml:\n");
    out.push_str(&plan.spec.to_yaml()?);
    Ok(out)
}

fn extract_javascript(content: &str) -> String {
    let trimmed = content.trim();
    if let Some(start) = trimmed.find("```") {
        let rest = &trimmed[start + 3..];
        let rest = rest
            .strip_prefix("javascript")
            .or_else(|| rest.strip_prefix("js"))
            .unwrap_or(rest);
        let rest = rest.trim_start();
        if let Some(end) = rest.find("```") {
            return rest[..end].trim().to_string();
        }
    }
    trimmed.to_string()
}
