// Dry-run plan extraction: QuickJS is the single grammar for workflow
// scripts. Validation IS execution against a recording host — a syntax error
// or policy violation surfaces as a hard error with the engine diagnostic,
// and the recorded typed calls are the approval-time plan preview.
//
// Included into workflow_live_v2_script.rs so it shares the script bridge
// (`script_source`) and the live host's typed payload parsing
// (`ScriptHostRequest`, `parse_script_options`) — one deserialization path
// for live and dry-run, no second interpretation of script source text.

const WORKFLOW_DRY_RUN_WATCHDOG: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkflowDryRunPlanDetails {
    pub(crate) calls: Vec<WorkflowV2HostCall>,
    /// (task id, write call id) pairs — coverage AND duplicate detection.
    pub(crate) write_task_claims: Vec<(String, String)>,
    /// Review map item claims captured from source items.
    pub(crate) review_map_claims: Vec<WorkflowReviewMapClaim>,
    /// Review reduce linkage and bounds captured from reviewContract metadata.
    pub(crate) review_reduce_edges: Vec<WorkflowReviewReduceEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkflowReviewMapClaim {
    pub(crate) review_kind: String,
    pub(crate) call_id: String,
    pub(crate) item_id: Option<String>,
    pub(crate) task_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkflowReviewReduceEdge {
    pub(crate) review_kind: String,
    pub(crate) call_id: String,
    pub(crate) stage: String,
    pub(crate) accounting_field: Option<String>,
    pub(crate) source_map_call_ids: Vec<String>,
    pub(crate) source_reduce_call_ids: Vec<String>,
    pub(crate) preserve_map_findings: bool,
    pub(crate) max_input_bytes: Option<usize>,
    pub(crate) max_findings_per_reduce: Option<usize>,
}

#[derive(Default)]
struct WorkflowDryRunRecorder {
    details: WorkflowDryRunPlanDetails,
    policy_error: Option<String>,
}

pub(crate) async fn dry_run_workflow_plan(
    harness_source: &str,
    script_args: Option<&serde_json::Value>,
) -> archon_workflow::WorkflowResult<Vec<WorkflowV2HostCall>> {
    dry_run_workflow_plan_details(harness_source, script_args)
        .await
        .map(|(calls, _)| calls)
}

/// Plan plus (task id, write call id) claims — the authoring pre-flight
/// requires every universe task claimed by exactly one write call.
pub(crate) async fn dry_run_workflow_plan_details(
    harness_source: &str,
    script_args: Option<&serde_json::Value>,
) -> archon_workflow::WorkflowResult<(Vec<WorkflowV2HostCall>, Vec<(String, String)>)> {
    let details = dry_run_workflow_plan_full_details(harness_source, script_args).await?;
    Ok((details.calls, details.write_task_claims))
}

pub(crate) async fn dry_run_workflow_plan_full_details(
    harness_source: &str,
    script_args: Option<&serde_json::Value>,
) -> archon_workflow::WorkflowResult<WorkflowDryRunPlanDetails> {
    let source = script_source(harness_source, script_args);
    tokio::task::spawn_blocking(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| {
                WorkflowError::SpecInvalid(format!(
                    "workflow.js dry-run local async runtime failed: {err}"
                ))
            })?;
        runtime.block_on(dry_run_on_current_thread(source))
    })
    .await
    .map_err(|err| WorkflowError::SpecInvalid(format!("workflow.js dry-run task failed: {err}")))?
}

async fn dry_run_on_current_thread(
    source: String,
) -> archon_workflow::WorkflowResult<WorkflowDryRunPlanDetails> {
    let recorder = Arc::new(StdMutex::new(WorkflowDryRunRecorder::default()));
    let runtime = AsyncRuntime::new()
        .map_err(|err| WorkflowError::SpecInvalid(format!("quickjs runtime failed: {err}")))?;
    let deadline = Instant::now() + WORKFLOW_DRY_RUN_WATCHDOG;
    runtime
        .set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)))
        .await;
    let context = AsyncContext::full(&runtime)
        .await
        .map_err(|err| WorkflowError::SpecInvalid(format!("quickjs context failed: {err}")))?;
    let recorder_for_js = recorder.clone();
    let js_result = context
        .async_with(async move |ctx| {
            ctx.globals().set(
                "__archonHost",
                Func::from(Async(move |method: String, payload: String| {
                    let recorder = recorder_for_js.clone();
                    async move {
                        record_dry_run_call(&recorder, &method, &payload).map_err(|err| {
                            rquickjs::Error::new_from_js_message(
                                "archon workflow dry-run host",
                                "string",
                                err.to_string(),
                            )
                        })
                    }
                })),
            )?;
            let promise: Promise = match ctx.eval(source.as_str()).catch(&ctx) {
                Ok(promise) => promise,
                Err(err) => {
                    return Err(rquickjs::Error::new_from_js_message(
                        "workflow.js",
                        "promise",
                        err.to_string(),
                    ));
                }
            };
            match promise.into_future::<String>().await.catch(&ctx) {
                Ok(result) => Ok(result),
                Err(err) => Err(rquickjs::Error::new_from_js_message(
                    "workflow.js",
                    "string",
                    err.to_string(),
                )),
            }
        })
        .await;
    let recorder = recorder
        .lock()
        .map_err(|_| WorkflowError::SpecInvalid("dry-run recorder lock poisoned".to_string()))?;
    // A policy violation is authoritative even if the script caught the throw.
    if let Some(policy_error) = &recorder.policy_error {
        return Err(WorkflowError::SpecInvalid(policy_error.clone()));
    }
    if let Err(err) = js_result {
        return Err(WorkflowError::SpecInvalid(format!(
            "workflow.js validation failed: {err}"
        )));
    }
    if recorder.details.calls.is_empty() {
        return Err(WorkflowError::SpecInvalid(
            "workflow.js declares no executable host calls".to_string(),
        ));
    }
    Ok(recorder.details.clone())
}

fn record_dry_run_call(
    recorder: &Arc<StdMutex<WorkflowDryRunRecorder>>,
    method: &str,
    payload: &str,
) -> archon_workflow::WorkflowResult<String> {
    let call = match dry_run_call_from_payload(method, payload) {
        Ok(call) => call,
        Err(err) => {
            record_policy_error(recorder, &err);
            return Err(err);
        }
    };
    let mut recorder = recorder
        .lock()
        .map_err(|_| WorkflowError::SpecInvalid("dry-run recorder lock poisoned".to_string()))?;
    if recorder.details.calls.iter().any(|seen| seen.id == call.id) {
        let error = format!("workflow.js has duplicate host call id `{}`", call.id);
        recorder.policy_error.get_or_insert(error.clone());
        return Err(WorkflowError::SpecInvalid(error));
    }
    let method = call.method;
    if call.write_mode.is_some()
        && let Ok(request) = serde_json::from_str::<ScriptHostRequest>(payload)
    {
        for item in request
            .source
            .as_ref()
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            // Same alias tolerance as the live credit path: every present
            // key contributes (no or_else shadowing), arrays or single
            // strings alike.
            for key in [
                "canonical_task_ids",
                "canonicalTaskIds",
                "canonical_task_id",
                "canonicalTaskId",
                "task_ids",
                "taskIds",
                "task_id",
                "taskId",
            ] {
                match item.get(key) {
                    Some(serde_json::Value::Array(ids)) => {
                        for id in ids.iter().filter_map(serde_json::Value::as_str) {
                            recorder
                                .details
                                .write_task_claims
                                .push((id.to_string(), call.id.clone()));
                        }
                    }
                    Some(serde_json::Value::String(id)) => {
                        recorder
                            .details
                            .write_task_claims
                            .push((id.clone(), call.id.clone()));
                    }
                    _ => {}
                }
            }
        }
    }
    record_review_contract_details(&mut recorder.details, &call, payload);
    recorder.details.calls.push(call);
    Ok(dry_run_stub_result(method))
}

fn record_review_contract_details(
    details: &mut WorkflowDryRunPlanDetails,
    call: &WorkflowV2HostCall,
    payload: &str,
) {
    let Some(contract) = review_contract_value(call) else {
        return;
    };
    let review_kind = contract
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let stage = contract
        .get("stage")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if review_kind.is_empty() || stage.is_empty() {
        return;
    }
    if stage == "map" {
        let Ok(request) = serde_json::from_str::<ScriptHostRequest>(payload) else {
            return;
        };
        for item in request
            .source
            .as_ref()
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            details.review_map_claims.push(WorkflowReviewMapClaim {
                review_kind: review_kind.clone(),
                call_id: call.id.clone(),
                item_id: item
                    .get("item_id")
                    .or_else(|| item.get("itemId"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                task_ids: task_ids_from_value(item),
            });
        }
    } else if stage == "reduce_final" || stage == "reduce_chunk" {
        details.review_reduce_edges.push(WorkflowReviewReduceEdge {
            review_kind,
            call_id: call.id.clone(),
            stage,
            accounting_field: contract
                .get("accountingField")
                .or_else(|| contract.get("accounting_field"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            source_map_call_ids: string_vec_contract(
                contract,
                &["sourceMapCallIds", "source_map_call_ids"],
            ),
            source_reduce_call_ids: string_vec_contract(
                contract,
                &["sourceReduceCallIds", "source_reduce_call_ids"],
            ),
            preserve_map_findings: contract
                .get("preserveMapFindings")
                .or_else(|| contract.get("preserve_map_findings"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            max_input_bytes: usize_contract(contract, &["maxInputBytes", "max_input_bytes"]),
            max_findings_per_reduce: usize_contract(
                contract,
                &["maxFindingsPerReduce", "max_findings_per_reduce"],
            ),
        });
    }
}

fn review_contract_value(call: &WorkflowV2HostCall) -> Option<&serde_json::Value> {
    call.options
        .extra
        .get("reviewContract")
        .or_else(|| call.options.extra.get("review_contract"))
}

fn string_vec_contract(value: &serde_json::Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn usize_contract(value: &serde_json::Value, keys: &[&str]) -> Option<usize> {
    keys.iter()
        .find_map(|key| value.get(*key))
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn task_ids_from_value(item: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    for key in [
        "canonical_task_ids",
        "canonicalTaskIds",
        "canonical_task_id",
        "canonicalTaskId",
        "task_ids",
        "taskIds",
        "task_id",
        "taskId",
    ] {
        match item.get(key) {
            Some(serde_json::Value::Array(ids)) => {
                out.extend(
                    ids.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(str::to_string),
                );
            }
            Some(serde_json::Value::String(id)) => {
                let id = id.trim();
                if !id.is_empty() {
                    out.push(id.to_string());
                }
            }
            _ => {}
        }
    }
    out
}

fn dry_run_call_from_payload(
    method: &str,
    payload: &str,
) -> archon_workflow::WorkflowResult<WorkflowV2HostCall> {
    let request: ScriptHostRequest = serde_json::from_str(payload)?;
    let method = WorkflowV2HostMethod::parse(method).ok_or_else(|| {
        WorkflowError::SpecInvalid(format!(
            "workflow.js used unsupported host method w.{method}"
        ))
    })?;
    reject_agent_routing_overrides(&request.id, &request.options)?;
    let (options, write_mode) = parse_script_options(&request.options)?;
    if write_mode.is_some() {
        reject_malformed_write_targets(&request.id, request.source.as_ref())?;
    }
    if method == WorkflowV2HostMethod::Implementation && write_mode.is_none() {
        return Err(WorkflowError::SpecInvalid(format!(
            "w.implementation('{}') requires explicit write mode serial, coordinated, or worktree",
            request.id
        )));
    }
    Ok(WorkflowV2HostCall {
        id: request.id,
        method,
        write_mode,
        options,
    })
}

// Policy: scripts must not steer provider routing. Enforced at the host
// boundary (dry-run records the violation; the live host parses the same
// payloads), not by scanning script source text.
fn reject_agent_routing_overrides(
    call_id: &str,
    options: &serde_json::Value,
) -> archon_workflow::WorkflowResult<()> {
    let Some(object) = options.as_object() else {
        return Ok(());
    };
    for key in ["model", "provider"] {
        if object.contains_key(key) {
            return Err(WorkflowError::SpecInvalid(format!(
                "host call '{call_id}' must not set `{key}`: provider routing is host policy"
            )));
        }
    }
    Ok(())
}

fn record_policy_error(
    recorder: &Arc<StdMutex<WorkflowDryRunRecorder>>,
    error: &archon_workflow::WorkflowError,
) {
    if let Ok(mut recorder) = recorder.lock() {
        recorder.policy_error.get_or_insert(error.to_string());
    }
}

// Policy: write items must declare literal repo-relative target file paths.
// Enforced at the host boundary (mirrors normalize_target: no whitespace, no
// traversal, no absolute paths; globs add nothing but late ownership
// mismatches) so prose targets fail the pre-flight even when script-side
// sugar is bypassed via raw w.fanout or a catch block.
fn reject_malformed_write_targets(
    call_id: &str,
    source: Option<&serde_json::Value>,
) -> archon_workflow::WorkflowResult<()> {
    let items = source.and_then(serde_json::Value::as_array);
    for item in items.into_iter().flatten() {
        let targets = item
            .get("target_files")
            .or_else(|| item.get("targetFiles"))
            .and_then(serde_json::Value::as_array);
        let Some(targets) = targets else { continue };
        for target in targets {
            let Some(target) = target.as_str() else {
                return Err(WorkflowError::SpecInvalid(format!(
                    "write call '{call_id}': target_files entries must be strings"
                )));
            };
            let trimmed = target.trim();
            let malformed = trimmed.is_empty()
                || trimmed.chars().any(char::is_whitespace)
                || trimmed.starts_with('/')
                || trimmed.split('/').any(|part| part == "..");
            if malformed {
                return Err(WorkflowError::SpecInvalid(format!(
                    "write call '{call_id}': target_files entries must be literal repo-relative file paths (got {trimmed:?})"
                )));
            }
            if trimmed.contains(['*', '?', '[']) {
                return Err(WorkflowError::SpecInvalid(format!(
                    "write call '{call_id}': target_files entry {trimmed:?} looks like a glob — ownership matching is literal, list the exact files"
                )));
            }
        }
    }
    Ok(())
}

fn dry_run_stub_result(method: WorkflowV2HostMethod) -> String {
    // The stub must carry the same envelope keys the live result view exposes
    // ({status, summary, data, result, ...}): reference-following scripts read
    // `x.result`/`x.data` fields, and a stub without them throws in the
    // pre-flight rehearsal, falsely rejecting a script that runs fine live.
    serde_json::json!({
        "status": "accepted",
        "summary": format!("dry-run stub result for w.{}", method.as_str()),
        "items": [],
        "outcomes": [],
        "data": {},
        "result": { "status": "accepted", "summary": "dry-run stub", "data": {} },
        "dry_run": true,
    })
    .to_string()
}

fn artifact_requirements(value: &serde_json::Value) -> Vec<WorkflowV2ArtifactRequirement> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| match entry {
            serde_json::Value::String(path) => {
                let path = path.trim();
                (!path.is_empty()).then(|| WorkflowV2ArtifactRequirement::new(path))
            }
            serde_json::Value::Object(object) => {
                let path = object.get("path")?.as_str()?.trim();
                if path.is_empty() {
                    return None;
                }
                let mut requirement = WorkflowV2ArtifactRequirement::new(path);
                requirement.kind = object
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                Some(requirement)
            }
            _ => None,
        })
        .collect()
}
