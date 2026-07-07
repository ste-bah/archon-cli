#[derive(Debug, serde::Deserialize)]
struct ScriptHostRequest {
    id: String,
    #[serde(default)]
    options: serde_json::Value,
    #[serde(default)]
    source: Option<serde_json::Value>,
}

fn parse_script_options(
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

fn string_value(value: &serde_json::Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_array(value: &serde_json::Value) -> Vec<String> {
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

fn script_source(harness_source: &str, script_args: Option<&serde_json::Value>) -> String {
    let normalized = normalize_workflow_export(harness_source);
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

async function __archonCall(method, id, source, options) {{
  if (typeof id !== "string" || id.trim() === "") {{
    throw new Error(`w.${{method}} requires a non-empty string id`);
  }}
  const payload = {{ id, options: options || {{}} }};
  if (source !== undefined) {{
    payload.source = source;
  }}
  const json = await __archonHost(method, JSON.stringify(payload));
  return JSON.parse(json);
}}

async function __archonRun() {{
  if (typeof workflow !== "function") {{
    throw new Error("workflow.js must export or define function workflow(w)");
  }}
  const result = await workflow(__archonW);
  return JSON.stringify(result ?? null);
}}

__archonRun()
"#
    )
}

fn normalize_workflow_export(source: &str) -> String {
    let trimmed = source.trim();
    if trimmed.starts_with("export default async function workflow") {
        return trimmed.replacen(
            "export default async function workflow",
            "async function workflow",
            1,
        );
    }
    if trimmed.starts_with("export default function workflow") {
        return trimmed.replacen("export default function workflow", "function workflow", 1);
    }
    if trimmed.starts_with("export default async function(") {
        return trimmed.replacen(
            "export default async function",
            "async function workflow",
            1,
        );
    }
    if trimmed.starts_with("export default function(") {
        return trimmed.replacen("export default function", "function workflow", 1);
    }
    trimmed
        .replace("export default workflow;", "")
        .replace("export default workflow", "")
}

fn result_view_json(result: &WorkflowV2Result) -> archon_workflow::WorkflowResult<String> {
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

fn completion_evidence_from_result(
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

fn reusable_record_has_required_completion_evidence(record: &WorkflowV2CallRecord) -> bool {
    !completion_evidence_call_id(&record.call.id) || !record.completion_evidence.is_empty()
}

fn evidence_snapshot_hash(evidence: &[WorkflowV2TaskCompletionEvidence]) -> Option<String> {
    if evidence.is_empty() {
        return None;
    }
    serde_json::to_value(evidence)
        .ok()
        .map(|value| stable_hash(&value))
}

fn completion_evidence_call_id(call_id: &str) -> bool {
    call_id.starts_with("noop-proof-verification-")
        || call_id.starts_with("noop-proof-reverification-")
        || call_id.starts_with("implementation-wave-")
        || call_id.starts_with("remediation-wave-")
        || call_id.starts_with("review-remediation-wave-")
        || call_id.starts_with("verification-wave-")
        || call_id.starts_with("review-verification-wave-")
}

fn is_reusable_status(status: WorkflowV2Status) -> bool {
    matches!(status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop)
}

fn terminal_stop_for_call(call: &WorkflowV2HostCall, status: WorkflowV2Status) -> bool {
    matches!(
        status,
        WorkflowV2Status::Failed | WorkflowV2Status::Cancelled
    ) || matches!(
        call.method,
        WorkflowV2HostMethod::HumanGate
            | WorkflowV2HostMethod::QualityGate
            | WorkflowV2HostMethod::RequireArtifact
            | WorkflowV2HostMethod::FinalReport
    ) && !matches!(status, WorkflowV2Status::Accepted | WorkflowV2Status::Noop)
}

fn merge_v2_status(left: WorkflowV2Status, right: WorkflowV2Status) -> WorkflowV2Status {
    if status_precedence(right) > status_precedence(left) {
        right
    } else {
        left
    }
}

fn status_precedence(status: WorkflowV2Status) -> u8 {
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

fn next_action_for_terminal_call(call_id: &str, status: WorkflowV2Status) -> String {
    match status {
        WorkflowV2Status::NeedsReview | WorkflowV2Status::Blocked => format!(
            "inspect the recorded result, choose a provided review action if present, then restart or resume: /workflow restart-stage <run-id> {call_id}"
        ),
        _ => format!(
            "choose one: /workflow restart-stage <run-id> {call_id}, /workflow restart-item <run-id> {call_id} <item-id> when a single branch failed, or fix the recorded evidence/artifact and /workflow resume --live <run-id>"
        ),
    }
}

fn normalize_result_for_call(
    execution: &WorkflowV2CallExecution,
    mut result: WorkflowV2Result,
) -> WorkflowV2Result {
    downgrade_read_only_accepted_task_coverage(&execution.call, &mut result);
    guard_empty_items_output(execution, &mut result);
    result
}

fn mark_unresolved_dependency_metadata(
    execution: &WorkflowV2CallExecution,
    metadata: &super::workflow_live_v2_source_graph::DynamicWaveSourceMetadata,
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

fn downgrade_read_only_accepted_task_coverage(
    call: &WorkflowV2HostCall,
    result: &mut WorkflowV2Result,
) {
    if call.write_mode.is_some() || call.method == WorkflowV2HostMethod::Implementation {
        return;
    }
    let has_implementation_evidence = !result.files_changed.is_empty()
        || result
            .evidence
            .iter()
            .any(|evidence| matches!(evidence.kind, WorkflowV2EvidenceKind::Implementation));
    let mut downgraded = Vec::new();
    for coverage in &mut result.task_coverage {
        if coverage.status != WorkflowV2TaskCoverageStatus::Accepted {
            continue;
        }
        let coverage_has_implementation_evidence = coverage.evidence.iter().any(|evidence| {
            matches!(
                evidence.kind,
                WorkflowV2EvidenceKind::Implementation | WorkflowV2EvidenceKind::Test
            )
        });
        if !has_implementation_evidence && !coverage_has_implementation_evidence {
            coverage.status = WorkflowV2TaskCoverageStatus::Unknown;
            coverage.evidence.push(WorkflowV2Evidence::new(
                WorkflowV2EvidenceKind::Review,
                "read-only workflow calls cannot accept implementation task coverage without concrete implementation or test evidence",
            ));
            downgraded.push(coverage.task_id.clone());
        }
    }
    if downgraded.is_empty() {
        return;
    }
    if matches!(result.status, WorkflowV2Status::Accepted) {
        result.status = WorkflowV2Status::NeedsReview;
    }
    result.evidence.push(WorkflowV2Evidence::new(
        WorkflowV2EvidenceKind::Review,
        format!(
            "downgraded read-only accepted task coverage to unknown for: {}",
            downgraded.join(", ")
        ),
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!("read_only_task_acceptance_{}", sanitize_v2_gap_id(&call.id)),
        description: "read-only call claimed implementation acceptance without concrete implementation/test evidence".to_string(),
        severity: Some("review".to_string()),
    });
}

fn guard_empty_items_output(execution: &WorkflowV2CallExecution, result: &mut WorkflowV2Result) {
    if !call_declares_items_output(execution) || !items_output_is_empty(result) {
        return;
    }
    if result
        .task_coverage
        .iter()
        .any(|coverage| coverage.status == WorkflowV2TaskCoverageStatus::Noop)
    {
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
        "items-producing call returned an empty data.items array without typed no-op proof",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "empty_items_output_{}",
            sanitize_v2_gap_id(&execution.call.id)
        ),
        description:
            "implementation inventory cannot be empty unless every required task has concrete no-op proof"
                .to_string(),
        severity: Some("review".to_string()),
    });
}

fn call_declares_items_output(execution: &WorkflowV2CallExecution) -> bool {
    execution
        .call
        .options
        .extra
        .get("outputs")
        .is_some_and(outputs_value_declares_items)
}

fn outputs_value_declares_items(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .any(|value| value.eq_ignore_ascii_case("items")),
        serde_json::Value::String(value) => value.eq_ignore_ascii_case("items"),
        _ => false,
    }
}

fn items_output_is_empty(result: &WorkflowV2Result) -> bool {
    result
        .data
        .get("items")
        .and_then(serde_json::Value::as_array)
        .is_some_and(Vec::is_empty)
}

fn sanitize_v2_gap_id(raw: &str) -> String {
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
