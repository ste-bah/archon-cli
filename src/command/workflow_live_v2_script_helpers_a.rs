use super::*;

#[derive(Debug, serde::Deserialize)]
pub(crate) struct ScriptHostRequest {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) options: serde_json::Value,
    #[serde(default)]
    pub(crate) source: Option<serde_json::Value>,
}

pub(crate) fn parse_script_options(
    value: &serde_json::Value,
) -> archon_workflow::WorkflowResult<(WorkflowV2HostOptions, Option<WorkflowV2WriteMode>)> {
    let mut options = WorkflowV2HostOptions::default();
    let mut write_mode = None;
    let Some(object) = value.as_object() else {
        if !value.is_null() {
            options.extra.insert("value".to_string(), value.clone());
        }
        return Ok((options, write_mode));
    };
    for (key, value) in object {
        match key.as_str() {
            "role" | "tier" => options.role = string_value(value),
            "task" => options.task = string_value(value),
            "source" => options.source = string_value(value),
            "itemKind" | "item_kind" => options.item_kind = string_value(value),
            "targetFiles" | "target_files" => {
                options.target_files = string_array(value);
            }
            "targetFilesFromItem" | "target_files_from_item" => {
                options.target_files_from_item = value.as_bool().unwrap_or(false);
            }
            "maxParallelism" | "max_parallelism" => {
                options.max_parallelism =
                    value.as_u64().and_then(|value| usize::try_from(value).ok());
            }
            "requiredArtifacts" | "required_artifacts" => {
                options.required_artifacts = artifact_requirements(value);
            }
            "write" | "writeMode" | "write_mode" => {
                if value.as_bool() == Some(true) {
                    return Err(WorkflowError::SpecInvalid(
                        "workflow.js write mode must be a string ('serial' | 'coordinated' | 'worktree'); `write: true` is only valid in the agent()/agents() primitives"
                            .to_string(),
                    ));
                }
                if let Some(raw) = value.as_str() {
                    if raw.eq_ignore_ascii_case("none") {
                        write_mode = None;
                    } else {
                        write_mode = Some(WorkflowV2WriteMode::parse(raw).ok_or_else(|| {
                            WorkflowError::SpecInvalid(format!(
                                "invalid workflow.js write mode '{raw}'"
                            ))
                        })?);
                    }
                }
            }
            _ => {
                options.extra.insert(key.clone(), value.clone());
            }
        }
    }
    Ok((options, write_mode))
}
pub(super) fn string_value(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
pub(super) fn string_array(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}
pub(crate) fn script_source(
    harness_source: &str,
    script_args: Option<&serde_json::Value>,
) -> String {
    let normalized = normalize_workflow_export(harness_source);
    let v3_primitives = V3_PRIMITIVES_JS;
    let args_literal = script_args
        .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "undefined".to_string()))
        .unwrap_or_else(|| "undefined".to_string());
    format!(
        r#"
globalThis.args = {args_literal};

// Determinism prelude: workflow scripts must be replayable. Wall-clock and
// randomness are host concerns; pass timestamps via args.
delete Math.random;
Math.random = () => {{
  throw new Error("Math.random() is unavailable in workflow scripts: workflows must be deterministic");
}};
const __archonRealDate = Date;
globalThis.Date = new Proxy(__archonRealDate, {{
  apply() {{
    throw new Error("Date() is unavailable in workflow scripts: pass timestamps via args");
  }},
  construct(target, argumentList) {{
    if (argumentList.length === 0) {{
      throw new Error("new Date() without arguments is unavailable in workflow scripts: pass timestamps via args");
    }}
    return new target(...argumentList);
  }},
  get(target, property, receiver) {{
    if (property === "now") {{
      return () => {{
        throw new Error("Date.now() is unavailable in workflow scripts: pass timestamps via args");
      }};
    }}
    return Reflect.get(target, property, receiver);
  }},
}});

{normalized}

const __archonW = Object.freeze({{
  agent: (id, options = {{}}) => __archonCall("agent", id, undefined, options),
  implementation: (id, options = {{}}) => __archonCall("implementation", id, undefined, options),
  fanout: (id, source, options = {{}}) => __archonCall("fanout", id, source, options),
  parallel: (id, source, options = {{}}) => __archonCall("parallel", id, source, options),
  tool: (id, options = {{}}) => __archonCall("tool", id, undefined, options),
  checkpoint: (id, options = {{}}) => __archonCall("checkpoint", id, undefined, options),
  saveArtifact: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("saveArtifact", id, sourceOrOptions, options),
  requireArtifact: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("requireArtifact", id, sourceOrOptions, options),
  reduce: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("reduce", id, sourceOrOptions, options),
  qualityGate: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("qualityGate", id, sourceOrOptions, options),
  humanGate: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("humanGate", id, sourceOrOptions, options),
  finalReport: (id, sourceOrOptions = {{}}, options) => __archonMaybeSourceCall("finalReport", id, sourceOrOptions, options),
}});

function __archonMaybeSourceCall(method, id, sourceOrOptions, options) {{
  if (options === undefined) {{
    return __archonCall(method, id, undefined, sourceOrOptions || {{}});
  }}
  return __archonCall(method, id, sourceOrOptions, options || {{}});
}}

// Every host call registers synchronously and deregisters on completion. A
// workflow that returns while calls are pending dropped real work on the
// floor (fire-and-forget async): fail closed, naming the dropped calls.
const __archonPendingCalls = new Set();

async function __archonCall(method, id, source, options) {{
  if (typeof id !== "string" || id.trim() === "") {{
    throw new Error(`w.${{method}} requires a non-empty string id`);
  }}
  const payload = {{ id, options: options || {{}} }};
  if (source !== undefined) {{
    payload.source = source;
  }}
  const pendingKey = `${{method}}:${{id}}`;
  __archonPendingCalls.add(pendingKey);
  try {{
    const json = await __archonHost(method, JSON.stringify(payload));
    return JSON.parse(json);
  }} finally {{
    __archonPendingCalls.delete(pendingKey);
  }}
}}

{v3_primitives}

async function __archonRun() {{
  if (typeof workflow !== "function") {{
    throw new Error("workflow.js must export or define function workflow(w)");
  }}
  const meta = typeof __workflowMeta !== "undefined" ? __workflowMeta : undefined;
  const api = meta ? __archonPrimitives(__archonW) : __archonW;
  if (meta) {{
    // Top-level Claude Code scripts use the primitives as bare globals.
    globalThis.agent = api.agent;
    globalThis.agents = api.agents;
    globalThis.phase = api.phase;
    globalThis.log = api.log;
    globalThis.pipeline = api.pipeline;
    globalThis.adversarialReview = api.adversarialReview;
    globalThis.coverageAudit = api.coverageAudit;
    globalThis.remediateFindings = api.remediateFindings;
    globalThis.remediationBudget = api.remediationBudget;
    globalThis.w = api.w;
  }}
  const result = await workflow(api);
  if (meta && globalThis.__archonMarkers) {{
    // phase()/log() markers need no await in scripts; the runner flushes
    // them so they are journaled before completion.
    await Promise.all(globalThis.__archonMarkers);
  }}
  if (__archonPendingCalls.size > 0) {{
    const dropped = [...__archonPendingCalls].join(", ");
    throw new Error(`workflow returned while ${{__archonPendingCalls.size}} host call(s) were still pending (${{dropped}}); await every agent call — fire-and-forget drops real work`);
  }}
  return JSON.stringify(result ?? null);
}}

__archonRun()
"#
    )
}
pub(crate) fn result_view_json(
    result: &WorkflowV2Result,
) -> archon_workflow::WorkflowResult<String> {
    let mut view = match &result.data {
        serde_json::Value::Object(object) => object.clone(),
        serde_json::Value::Null => serde_json::Map::new(),
        value => {
            let mut object = serde_json::Map::new();
            object.insert("data".to_string(), value.clone());
            object
        }
    };
    view.insert("status".to_string(), serde_json::to_value(result.status)?);
    view.insert(
        "summary".to_string(),
        serde_json::Value::String(result.summary.clone()),
    );
    view.insert("result".to_string(), serde_json::to_value(result)?);
    serde_json::to_string(&serde_json::Value::Object(view)).map_err(Into::into)
}
pub(crate) fn completion_evidence_from_result(
    result: &WorkflowV2Result,
) -> Vec<WorkflowV2TaskCompletionEvidence> {
    let mut evidence = Vec::new();
    let Some(outcomes) = result
        .data
        .get("outcomes")
        .and_then(serde_json::Value::as_array)
    else {
        return evidence;
    };
    for outcome in outcomes {
        let Some(items) = outcome
            .get("completion_evidence")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for item in items {
            if let Ok(parsed) =
                serde_json::from_value::<WorkflowV2TaskCompletionEvidence>(item.clone())
            {
                evidence.push(parsed);
            }
        }
    }
    evidence
}
pub(crate) fn reusable_record_has_required_completion_evidence(
    record: &WorkflowV2CallRecord,
) -> bool {
    !completion_evidence_call_id(&record.call.id) || !record.completion_evidence.is_empty()
}

/// True when every task this record covers is in the restart's completed set
/// (and it covers at least one). Used to reuse an already-accepted call —
/// including its verification — on `restart task <id>` without re-checking
/// scaffold/input hashes, so tasks before the restart point are not
/// re-validated from the top.
pub(crate) fn record_tasks_all_completed(
    record: &WorkflowV2CallRecord,
    completed: &std::collections::BTreeSet<String>,
) -> bool {
    if completed.is_empty() {
        return false;
    }
    let mut tasks: std::collections::BTreeSet<&str> =
        record.completed_ids.iter().map(String::as_str).collect();
    for evidence in &record.completion_evidence {
        let task_id = evidence.task_id.trim();
        if !task_id.is_empty() {
            tasks.insert(task_id);
        }
    }
    !tasks.is_empty() && tasks.iter().all(|task| completed.contains(*task))
}

/// The v3 authored-script call family a call id belongs to, or None for
/// anything else (decomposed `implementation-wave-*`/`verification-wave-N`,
/// reviews, discovery). Reuse across ordinal drift must only match within the
/// same family so a stale decomposed record can never satisfy a v3 task call.
#[derive(PartialEq, Eq, Clone, Copy)]
pub(crate) enum V3CallFamily {
    Implement,
    Verify,
}

pub(crate) fn v3_call_family(call_id: &str) -> Option<V3CallFamily> {
    let id = call_id.to_ascii_lowercase();
    if id.starts_with("implement-task-") || id.starts_with("remediate-task-") {
        Some(V3CallFamily::Implement)
    } else if id.starts_with("verification-wave-verify-task-") {
        Some(V3CallFamily::Verify)
    } else {
        None
    }
}

/// Whether a resume may adopt an accepted record from the frontier.
///
/// This path is laxer than strict reuse about the dynamic source fingerprint
/// (the caller waives it for calls that require no source metadata), but it is
/// NOT laxer about the input: it used to ignore `input_hash` entirely, so a
/// resume replayed a recorded result for a call whose input had since changed —
/// there is no invalidation pass that would have caught it, because
/// `invalidate_*` only ever runs from the operator's `workflow restart`
/// command. Content keying is the whole safety argument for reuse, so the
/// frontier path keys on content too.
pub(in super::super::super) fn frontier_resume_record_reusable(
    record: &WorkflowV2CallRecord,
    input_hash: &str,
    scaffold_hash: &str,
) -> bool {
    record.is_reusable_for(input_hash) && record.scaffold_hash.as_deref() == Some(scaffold_hash)
}

pub(crate) fn evidence_snapshot_hash(
    evidence: &[WorkflowV2TaskCompletionEvidence],
) -> Option<String> {
    if evidence.is_empty() {
        return None;
    }
    serde_json::to_value(evidence)
        .ok()
        .map(|value| stable_hash(&value))
}

pub(super) fn completion_evidence_call_id(call_id: &str) -> bool {
    call_id.starts_with("noop-proof-verification-")
        || call_id.starts_with("noop-proof-reverification-")
        || call_id.starts_with("implementation-wave-")
        || call_id.starts_with("remediation-wave-")
        || call_id.starts_with("review-remediation-wave-")
        || call_id.starts_with("verification-wave-")
        || call_id.starts_with("review-verification-wave-")
}

pub(crate) fn is_reusable_status(status: WorkflowV2Status) -> bool {
    matches!(status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop)
}

pub(crate) fn terminal_stop_for_call(call: &WorkflowV2HostCall, status: WorkflowV2Status) -> bool {
    // Errors are values: task-level failures flow back to the script as
    // structured results for script-owned remediation. Only cancellation and
    // unsatisfied final/human gates unwind the script.
    matches!(status, WorkflowV2Status::Cancelled)
        || matches!(
            call.method,
            WorkflowV2HostMethod::HumanGate | WorkflowV2HostMethod::FinalReport
        ) && !matches!(status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop)
}

pub(crate) fn merge_v2_status(left: WorkflowV2Status, right: WorkflowV2Status) -> WorkflowV2Status {
    if status_precedence(right) > status_precedence(left) {
        right
    } else {
        left
    }
}

pub(super) fn status_precedence(status: WorkflowV2Status) -> u8 {
    match status {
        WorkflowV2Status::Cancelled => 7,
        WorkflowV2Status::Failed => 6,
        WorkflowV2Status::Blocked => 5,
        WorkflowV2Status::NeedsReview => 4,
        WorkflowV2Status::Running => 3,
        WorkflowV2Status::Pending => 2,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop => 1,
    }
}

/// The severity a non-final stage contributes to the RUN's terminal status.
///
/// A stage that failed on a transport/compaction/infrastructure error — not a
/// genuine work rejection — must not doom an otherwise-complete run to `Failed`
/// and discard every honest task outcome behind an infra hiccup. Such a stage
/// contributes `Blocked` instead: honest ("this stage could not complete and is
/// resumable") and non-fatal, so the run terminates reflecting the real task
/// tally with the stage flagged, and can be resumed to re-run just that stage.
/// The stage record itself stays truthful — still `Failed` with the transport
/// reason. Genuine (non-infrastructure) failures contribute `Failed` unchanged.
/// General: keys only on the failure text via the shared transport detector, so
/// it holds for any stage, PRD, tool, or workflow — no special cases.
pub(crate) fn run_terminal_status_contribution(
    record: &WorkflowV2CallRecord,
    status: WorkflowV2Status,
) -> WorkflowV2Status {
    if status == WorkflowV2Status::Failed && is_transport_failure_text(&record.result.summary) {
        WorkflowV2Status::Blocked
    } else {
        status
    }
}

pub(crate) fn next_action_for_terminal_call(call_id: &str, status: WorkflowV2Status) -> String {
    match status {
        WorkflowV2Status::NeedsReview | WorkflowV2Status::Blocked => format!(
            "inspect the recorded result, choose a provided review action if present, then restart or resume: /workflow restart-stage <run-id> {call_id}"
        ),
        _ => format!(
            "choose one: /workflow restart-stage <run-id> {call_id}, /workflow restart-item <run-id> {call_id} <item-id> when a single branch failed, or fix the recorded evidence/artifact and /workflow resume --live <run-id>"
        ),
    }
}

pub(crate) fn normalize_result_for_call(
    execution: &WorkflowV2CallExecution,
    mut result: WorkflowV2Result,
) -> WorkflowV2Result {
    archon_workflow::v2::artifact_refs::retain_filesystem_artifacts(&mut result);
    downgrade_read_only_accepted_task_coverage(&execution.call, &mut result);
    guard_empty_items_output(execution, &mut result);
    result
}

pub(crate) fn mark_unresolved_dependency_metadata(
    execution: &WorkflowV2CallExecution,
    metadata: &archon_workflow::v2::source_graph::DynamicWaveSourceMetadata,
    result: &mut WorkflowV2Result,
) {
    if metadata.unresolved_dependencies.is_empty() {
        return;
    }
    if matches!(
        result.status,
        WorkflowV2Status::Accepted | WorkflowV2Status::Noop
    ) {
        result.status = WorkflowV2Status::NeedsReview;
    }
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        format!(
            "dynamic implementation wave '{}' has unresolved dependency references: {}",
            execution.call.id,
            metadata.unresolved_dependencies.join(", ")
        ),
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "unresolved_dynamic_wave_dependencies_{}",
            sanitize_v2_gap_id(&execution.call.id)
        ),
        description: format!(
            "unresolved dependency references prevent reusable source fingerprint: {}",
            metadata.unresolved_dependencies.join(", ")
        ),
        severity: Some("review".to_string()),
    });
}
