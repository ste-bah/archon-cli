fn pipeline_step_action_id(
    parent_action_id: &str,
    ordinal: usize,
    agent: &archon_pipeline::runner::AgentInfo,
) -> String {
    format!("{parent_action_id}:step:{ordinal}:{}", agent.key)
}

fn pipeline_step_summary(
    agent: &archon_pipeline::runner::AgentInfo,
    result: &archon_pipeline::runner::AgentResult,
    ordinal: usize,
) -> String {
    let quality = result
        .quality
        .as_ref()
        .map(|quality| format!("{:.2}", quality.overall))
        .unwrap_or_else(|| "unscored".into());
    format!(
        "pipeline step {ordinal}: phase={} agent={} critical={} quality={quality} threshold={:.2}",
        agent.phase, agent.key, agent.critical, agent.quality_threshold
    )
}

fn pipeline_step_outcome_summary(
    agent: &archon_pipeline::runner::AgentInfo,
    result: &archon_pipeline::runner::AgentResult,
) -> String {
    let quality = result
        .quality
        .as_ref()
        .map(|quality| format!("{:.2}", quality.overall))
        .unwrap_or_else(|| "unscored".into());
    format!(
        "pipeline agent {} completed; quality={quality}; duration_ms={}; tokens_in={}; tokens_out={}",
        agent.key,
        result.duration.as_millis(),
        result.tokens_in,
        result.tokens_out
    )
}

fn pipeline_agent_quality_failed(
    agent: &archon_pipeline::runner::AgentInfo,
    result: &archon_pipeline::runner::AgentResult,
) -> bool {
    result
        .quality
        .as_ref()
        .is_some_and(|quality| quality.overall < agent.quality_threshold)
}

fn pipeline_agent_verification(
    parent: &RuntimeGuardrailRecord,
    step_action_id: &str,
    agent: &archon_pipeline::runner::AgentInfo,
    result: &archon_pipeline::runner::AgentResult,
) -> Option<archon_world_model::VerificationOutcome> {
    if let Some(signal) = result
        .tool_use_log
        .iter()
        .find_map(pipeline_tool_verification_signal)
    {
        return Some(pipeline_verification_outcome(
            parent,
            step_action_id,
            agent,
            signal,
        ));
    }

    if !pipeline_agent_has_verification_hint(agent) {
        return None;
    }
    Some(pipeline_verification_outcome(
        parent,
        step_action_id,
        agent,
        PipelineVerificationSignal {
            kind: archon_world_model::VerificationKind::Custom("verifier".into()),
            status: archon_world_model::VerificationStatus::Inconclusive,
            command: Some(format!("pipeline-agent:{}", agent.key)),
            exit_code: None,
            summary: "no_execution_signal: agent looked verification-like, but no structured test/build/verifier execution result was surfaced".into(),
            evidence_refs: vec![
                "guardrail:no_execution_signal".into(),
                format!("pipeline_agent_hint:{}", agent.key),
            ],
        },
    ))
}

#[derive(Debug, Clone)]
struct PipelineVerificationSignal {
    kind: archon_world_model::VerificationKind,
    status: archon_world_model::VerificationStatus,
    command: Option<String>,
    exit_code: Option<i32>,
    summary: String,
    evidence_refs: Vec<String>,
}

fn pipeline_verification_outcome(
    parent: &RuntimeGuardrailRecord,
    step_action_id: &str,
    agent: &archon_pipeline::runner::AgentInfo,
    signal: PipelineVerificationSignal,
) -> archon_world_model::VerificationOutcome {
    archon_world_model::VerificationOutcome {
        schema_version: archon_world_model::guardrail::CURRENT_SCHEMA_VERSION,
        requirement_id: matching_requirement_id(&parent.action, &signal.kind),
        action_id: parent.action.action_id.clone(),
        kind: signal.kind,
        status: signal.status,
        command: signal.command,
        exit_code: signal.exit_code,
        summary: signal.summary,
        evidence_refs: {
            let mut refs = vec![
                format!("guardrail_pipeline_step:{step_action_id}"),
                format!("pipeline_agent:{}", agent.key),
            ];
            refs.extend(signal.evidence_refs);
            refs.sort();
            refs.dedup();
            refs
        },
        idempotency_key: format!(
            "world_guardrail:pipeline_verification:{}:{}",
            parent.action.action_id, step_action_id
        ),
        created_at: chrono::Utc::now(),
    }
}

fn pipeline_tool_verification_signal(
    entry: &archon_pipeline::runner::ToolUseEntry,
) -> Option<PipelineVerificationSignal> {
    let command = pipeline_tool_command(entry)?;
    let (_, kind) = classify_tool_command(&command);
    let kind = kind?;
    let signal = execution_signal_from_tool_output(&entry.output);
    let (status, exit_code, signal_summary, mut evidence_refs) = signal.unwrap_or_else(|| {
        (
            archon_world_model::VerificationStatus::Inconclusive,
            None,
            "no_execution_signal: verification command was planned, but no structured execution result was surfaced".into(),
            vec!["guardrail:no_execution_signal".into()],
        )
    });
    evidence_refs.push(format!("pipeline_tool:{}", entry.tool_name));
    Some(PipelineVerificationSignal {
        kind,
        status,
        command: Some(command),
        exit_code,
        summary: signal_summary,
        evidence_refs,
    })
}

fn pipeline_tool_command(entry: &archon_pipeline::runner::ToolUseEntry) -> Option<String> {
    if let Some(command) = entry.input.as_str() {
        return non_empty(command);
    }
    let object = entry.input.as_object()?;
    for key in ["command", "cmd", "script", "shell_command"] {
        if let Some(command) = object.get(key).and_then(|value| value.as_str()) {
            return non_empty(command);
        }
    }
    object
        .get("args")
        .and_then(|value| value.as_array())
        .and_then(|args| {
            let joined = args
                .iter()
                .filter_map(|value| value.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            non_empty(&joined)
        })
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn execution_signal_from_tool_output(
    output: &serde_json::Value,
) -> Option<(
    archon_world_model::VerificationStatus,
    Option<i32>,
    String,
    Vec<String>,
)> {
    if output.is_null() {
        return None;
    }
    if let Some(exit_code) = find_i64_field(output, &["exit_code", "code", "status_code"]) {
        let exit_code = exit_code as i32;
        let status = if exit_code == 0 {
            archon_world_model::VerificationStatus::Passed
        } else {
            archon_world_model::VerificationStatus::Failed
        };
        return Some((
            status,
            Some(exit_code),
            execution_summary(output, &format!("exit_code={exit_code}")),
            vec!["guardrail:execution_exit_code".into()],
        ));
    }
    if let Some(success) = find_bool_field(output, &["success", "passed", "ok"]) {
        let status = if success {
            archon_world_model::VerificationStatus::Passed
        } else {
            archon_world_model::VerificationStatus::Failed
        };
        return Some((
            status,
            None,
            execution_summary(output, &format!("success={success}")),
            vec!["guardrail:execution_success_bool".into()],
        ));
    }
    if let Some(is_error) = find_bool_field(output, &["is_error", "error"]) {
        let status = if is_error {
            archon_world_model::VerificationStatus::Failed
        } else {
            archon_world_model::VerificationStatus::Passed
        };
        return Some((
            status,
            None,
            execution_summary(output, &format!("is_error={is_error}")),
            vec!["guardrail:execution_error_bool".into()],
        ));
    }
    if let Some(status) = find_string_field(output, &["status", "outcome", "conclusion"]) {
        let normalized = status.to_ascii_lowercase();
        let mapped = match normalized.as_str() {
            "passed" | "pass" | "success" | "succeeded" | "ok" | "completed" => {
                archon_world_model::VerificationStatus::Passed
            }
            "failed" | "fail" | "failure" | "error" | "errored" => {
                archon_world_model::VerificationStatus::Failed
            }
            "inconclusive" | "unknown" | "skipped" | "not_run" => {
                archon_world_model::VerificationStatus::Inconclusive
            }
            _ => return None,
        };
        return Some((
            mapped,
            None,
            execution_summary(output, &format!("status={status}")),
            vec!["guardrail:execution_status_string".into()],
        ));
    }
    None
}

fn find_i64_field(value: &serde_json::Value, keys: &[&str]) -> Option<i64> {
    let object = value.as_object()?;
    for key in keys {
        if let Some(value) = object.get(*key).and_then(|value| value.as_i64()) {
            return Some(value);
        }
    }
    None
}

fn find_bool_field(value: &serde_json::Value, keys: &[&str]) -> Option<bool> {
    let object = value.as_object()?;
    for key in keys {
        if let Some(value) = object.get(*key).and_then(|value| value.as_bool()) {
            return Some(value);
        }
    }
    None
}

fn find_string_field(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    let object = value.as_object()?;
    for key in keys {
        if let Some(value) = object.get(*key).and_then(|value| value.as_str()) {
            return Some(value.to_string());
        }
    }
    None
}

fn execution_summary(output: &serde_json::Value, fallback: &str) -> String {
    if let Some(summary) = find_string_field(output, &["summary", "message", "stdout", "stderr"]) {
        return summary.trim().chars().take(500).collect();
    }
    fallback.to_string()
}

fn pipeline_agent_has_verification_hint(agent: &archon_pipeline::runner::AgentInfo) -> bool {
    let name = format!("{} {}", agent.key, agent.display_name).to_ascii_lowercase();
    name.contains("verify")
        || name.contains("verifier")
        || name.contains("review")
        || name.contains("approver")
        || name.contains("test")
        || name.contains("build")
        || name.contains("compile")
        || name.contains("lint")
        || name.contains("typecheck")
        || name.contains("type-check")
        || name.contains("source")
        || name.contains("citation")
        || name.contains("fact")
        || name.contains("security")
        || name.contains("static")
}

pub(crate) fn record_guardrail_provider_incident_for_session(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    provider_event_id: &str,
    reason_code: &str,
) -> bool {
    if !config.learning.world_model.guardrails.enabled {
        return false;
    }
    let Some(parent) = active_guardrail_for_session(session_id) else {
        return false;
    };
    let mut observations = active_observations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = observations
        .entry(parent.action.action_id.clone())
        .or_default();
    entry.provider_incident_observed = true;
    if provider_reason_implies_retry(reason_code) {
        entry.retry_count = entry.retry_count.saturating_add(1);
    }
    entry
        .evidence_refs
        .push(format!("provider_event:{provider_event_id}"));
    entry
        .evidence_refs
        .push(format!("provider_reason:{reason_code}"));
    entry.evidence_refs.sort();
    entry.evidence_refs.dedup();
    true
}

