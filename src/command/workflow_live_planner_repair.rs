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
    generated_config: &GeneratedWorkflowConfig,
) -> Result<WorkflowScriptPlan> {
    let task_universe = extract_task_universe_for_generated_run(task)?;
    if let Some(task_universe) = task_universe {
        let target_repository_root = infer_target_repository_root(task, Some(&task_universe));
        let governed_learning_context = recent_generated_learning_context(store, 8);
        let _ = tui_tx.send(TuiEvent::TextDelta(
            "Workflow planner: generating deterministic decomposed-PRD workflow.js scaffold; provider output cannot alter orchestration.\n"
                .into(),
        ));
        let harness = decomposed_prd_scaffold(
            task,
            target_repository_root.as_deref(),
            &task_universe,
            &governed_learning_context,
            generated_config,
        )?;
        match compile_harness_plan(task, Some(task_universe), &harness, generated_config).await {
            Ok(mut plan) => {
                plan.governed_learning_context = governed_learning_context;
                let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                    "Workflow planner: validated deterministic V2 scaffold '{}' with {} host call(s); creating run...\n",
                    plan.name,
                    plan.calls.len()
                )));
                return Ok(plan);
            }
            Err(err) => {
                let error = err.to_string();
                let attempt = planner_attempt("scaffold", 1, Some(&error), &harness);
                let message = planner_failure_message(store, task, &error, &[attempt]);
                let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                    "Workflow planner failed deterministic scaffold validation; live mode will not fall back to provider-authored orchestration: {message}\n"
                )));
                return Err(anyhow!(message));
            }
        }
    }

    let _ = tui_tx.send(TuiEvent::TextDelta(
        "Workflow planner: requesting workflow.js harness from active provider; no run directory exists until validation passes.\n"
            .into(),
    ));
    match llm_plan(task, None, llm, &tui_tx, generated_config).await {
        Ok(plan) => {
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "Workflow planner: validated V2 harness '{}' with {} host call(s); creating run...\n",
                plan.name,
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

fn recent_generated_learning_context(
    store: &WorkflowStore,
    limit: usize,
) -> Vec<GeneratedWorkflowLearningContext> {
    let Ok(runs) = store.list_runs() else {
        return Vec::new();
    };
    let mut context = Vec::new();
    for run in runs {
        let path = store
            .run_dir(&run.id)
            .join("learning/generated-workflow-events.jsonl");
        let Ok(raw) = fs::read_to_string(&path) else {
            continue;
        };
        for line in raw.lines().rev() {
            if line.trim().is_empty() {
                continue;
            }
            let Ok(mut event) = serde_json::from_str::<WorkflowLearningEvent>(line) else {
                continue;
            };
            if event.run_id.is_empty() {
                event.run_id = run.id.clone();
            }
            context.push(GeneratedWorkflowLearningContext::from_event(&event));
            if context.len() >= limit {
                return context;
            }
        }
    }
    context
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
    task_universe: Option<WorkflowV2TaskUniverse>,
    llm: Arc<dyn LlmClient>,
    tui_tx: &TuiEventSender,
    generated_config: &GeneratedWorkflowConfig,
) -> std::result::Result<WorkflowScriptPlan, PlannerFailure> {
    let response = workflow_live_retry::send_message_with_transient_retry(
        &llm,
        vec![serde_json::json!({
            "role": "user",
            "content": harness_planner_prompt(task, task_universe.as_ref()),
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
    validate_or_repair_harness(task, task_universe, raw, llm, tui_tx, generated_config).await
}

async fn validate_or_repair_harness(
    task: &str,
    task_universe: Option<WorkflowV2TaskUniverse>,
    raw: String,
    llm: Arc<dyn LlmClient>,
    tui_tx: &TuiEventSender,
    generated_config: &GeneratedWorkflowConfig,
) -> std::result::Result<WorkflowScriptPlan, PlannerFailure> {
    const MAX_REPAIRS: usize = 2;
    let mut harness = raw;
    let mut attempts = Vec::new();
    for attempt in 0..=MAX_REPAIRS {
        match compile_harness_plan(task, task_universe.clone(), &harness, generated_config).await {
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
                harness = request_repaired_harness(
                    task,
                    task_universe.as_ref(),
                    &harness,
                    error,
                    llm.clone(),
                )
                .await
                .map_err(|err| PlannerFailure::with_attempts(err.to_string(), attempts.clone()))?;
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

async fn compile_harness_plan(
    task: &str,
    task_universe: Option<WorkflowV2TaskUniverse>,
    harness_source: &str,
    generated_config: &GeneratedWorkflowConfig,
) -> archon_workflow::WorkflowResult<WorkflowScriptPlan> {
    let calls = if task_universe.is_some() {
        // Native lifecycle: the plan is declared by the Rust generator and
        // the recorded document is a descriptor, not an executable script.
        super::workflow_live_generated_scaffold::decomposed_prd_plan_calls()
    } else {
        // QuickJS is the single grammar for LLM-authored scripts: the dry-run
        // compiles the script and records its typed host calls.
        super::workflow_live_v2::dry_run_workflow_plan(harness_source, None).await?
    };
    Ok(WorkflowScriptPlan::generated(
        task,
        harness_source,
        calls,
        task_universe,
        generated_config.clone(),
    ))
}

async fn request_repaired_harness(
    task: &str,
    task_universe: Option<&WorkflowV2TaskUniverse>,
    invalid_harness: &str,
    error: String,
    llm: Arc<dyn LlmClient>,
) -> Result<String> {
    let response = workflow_live_retry::send_message_with_transient_retry(
        &llm,
        vec![serde_json::json!({
            "role": "user",
            "content": harness_repair_prompt(task, task_universe, invalid_harness, &error),
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
