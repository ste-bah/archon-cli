use std::path::Path;

use archon_tools::tool::{ToolRunAdmission, ToolRunAdmissionRequest, ToolRunAttemptOutcome};

pub(crate) fn admit_tool_run_attempt(
    config: &archon_core::config::ArchonConfig,
    request: ToolRunAdmissionRequest,
) -> ToolRunAdmission {
    admit_tool_run_attempt_with_root(config, super::world_model_root(), request)
}

fn admit_tool_run_attempt_with_root(
    config: &archon_core::config::ArchonConfig,
    root: anyhow::Result<std::path::PathBuf>,
    request: ToolRunAdmissionRequest,
) -> ToolRunAdmission {
    let policy = policy_from_config(config);
    let mode = archon_world_model::guardrail::mode_for_surface(
        &policy,
        archon_world_model::integration::WorldAdvisorSurface::ToolRun,
    );
    if !policy.enabled || matches!(mode, archon_world_model::WorldGuardrailMode::Off) {
        return ToolRunAdmission::Allowed;
    }
    let Ok(root) = root else {
        return if mode.can_block() {
            tool_run_storage_block("world-model root is unavailable")
        } else {
            ToolRunAdmission::Allowed
        };
    };
    admit_tool_run_attempt_at_root(config, &root, request)
}

fn admit_tool_run_attempt_at_root(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    request: ToolRunAdmissionRequest,
) -> ToolRunAdmission {
    let policy = policy_from_config(config);
    let mode = archon_world_model::guardrail::mode_for_surface(
        &policy,
        archon_world_model::integration::WorldAdvisorSurface::ToolRun,
    );
    if !policy.enabled || matches!(mode, archon_world_model::WorldGuardrailMode::Off) {
        return ToolRunAdmission::Allowed;
    }

    let started_at = std::time::Instant::now();
    if persist_tool_run_candidate_at_root(root, &request).is_err() {
        return if mode.can_block() {
            tool_run_storage_block("candidate trace could not be persisted")
        } else {
            ToolRunAdmission::Allowed
        };
    }
    let action_id = tool_run_action_id(&request);
    let summary = tool_run_summary(&request);
    let advisory = super::runtime::record_runtime_advisory(
        config,
        archon_world_model::integration::WorldAdvisorSurface::ToolRun,
        &request.session_id,
        &action_id,
        &summary,
    );
    admit_tool_run_with_policy_at_root(root, &request, policy, mode, advisory, started_at)
}

#[cfg(test)]
fn admit_tool_run_at_root(
    config: &archon_core::config::ArchonConfig,
    root: &Path,
    request: &ToolRunAdmissionRequest,
    advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord,
) -> ToolRunAdmission {
    let policy = policy_from_config(config);
    let mode = archon_world_model::guardrail::mode_for_surface(
        &policy,
        archon_world_model::integration::WorldAdvisorSurface::ToolRun,
    );
    if !policy.enabled || matches!(mode, archon_world_model::WorldGuardrailMode::Off) {
        return ToolRunAdmission::Allowed;
    }

    admit_tool_run_with_policy_at_root(
        root,
        request,
        policy,
        mode,
        advisory,
        std::time::Instant::now(),
    )
}

fn admit_tool_run_with_policy_at_root(
    root: &Path,
    request: &ToolRunAdmissionRequest,
    policy: archon_world_model::WorldGuardrailPolicyConfig,
    mode: archon_world_model::WorldGuardrailMode,
    advisory: archon_world_model::integration::WorldAdvisorSurfaceRecord,
    started_at: std::time::Instant,
) -> ToolRunAdmission {
    let mut action = tool_run_action(request);
    let task_class = archon_world_model::classify_tool_action(
        &request.tool_name,
        &request.input,
        archon_world_model::integration::WorldAdvisorSurface::ToolRun,
    );
    let decision = if advisory.prediction.is_some() {
        let scores = guardrail_scores_for_prediction(task_class, advisory.prediction.as_ref());
        let context = archon_world_model::WorldGuardrailPredictionContext::from_scores(
            task_class, mode, scores, &policy,
        );
        archon_world_model::guardrail::decide_guardrail(
            &action,
            advisory.prediction.as_ref(),
            context,
            &policy,
        )
    } else {
        archon_world_model::WorldGuardrailDecision::unavailable(&action)
    };
    let decision = archon_world_model::guardrail::enforce_guardrail_overhead_budget(
        decision,
        elapsed_ms_u64(started_at),
        policy.max_guardrail_overhead_ms,
    );
    action.verification_plan = verification_plan_for_decision(&action.action_id, &decision);
    let revision_key = format!("world_guardrail:revision:{}:initial", action.action_id);
    if archon_world_model::guardrail::append_guardrail_revision(
        root,
        action,
        decision.clone(),
        revision_key,
    )
    .is_err()
    {
        return if mode.can_block() {
            tool_run_storage_block("admission storage could not be persisted")
        } else {
            ToolRunAdmission::Allowed
        };
    }

    if decision.allowed_to_continue {
        ToolRunAdmission::Allowed
    } else {
        ToolRunAdmission::Blocked {
            reason: tool_run_block_reason(&decision),
        }
    }
}

fn tool_run_storage_block(reason: &str) -> ToolRunAdmission {
    ToolRunAdmission::Blocked {
        reason: format!("ToolRun guardrail admission storage failure: {reason}"),
    }
}

pub(crate) fn record_tool_run_attempt_outcome(attempt: ToolRunAttemptOutcome) {
    let Ok(root) = super::world_model_root() else {
        return;
    };
    record_tool_run_attempt_outcome_at_root(&root, attempt);
}

fn record_tool_run_attempt_outcome_at_root(root: &Path, attempt: ToolRunAttemptOutcome) {
    let action_id = tool_run_action_id_parts(
        &attempt.parent_action_id,
        &attempt.tool_use_id,
        attempt.attempt,
    );
    let decisions = match archon_world_model::guardrail::load_guardrail_decisions(root) {
        Ok(decisions) => decisions,
        Err(error) => {
            tracing::warn!(%error, %action_id, "failed to load ToolRun admission decision");
            record_unavailable_tool_run_outcome(
                root,
                &attempt,
                &action_id,
                "guardrail_decision_unavailable:store_unavailable",
            );
            return;
        }
    };
    let decision = decisions
        .into_iter()
        .find(|decision| decision.action_id == action_id);
    let Some(decision) = decision else {
        tracing::warn!(%action_id, "ToolRun outcome has no persisted admission decision");
        record_unavailable_tool_run_outcome(
            root,
            &attempt,
            &action_id,
            "guardrail_decision_unavailable:not_found",
        );
        return;
    };
    let task_class = archon_world_model::classify_tool_action(
        &attempt.tool_name,
        &attempt.input,
        archon_world_model::integration::WorldAdvisorSurface::ToolRun,
    );
    let final_status = if attempt.blocked || attempt.is_error {
        archon_world_model::GuardrailFinalStatus::Failed
    } else {
        archon_world_model::GuardrailFinalStatus::CompletedWithCaveat
    };
    let mut outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
        &decision,
        task_class,
        final_status,
        if attempt.blocked {
            "ToolRun blocked before execution"
        } else if attempt.is_error {
            "ToolRun execution failed"
        } else {
            "ToolRun execution completed"
        },
    );
    outcome.outcome_id = format!("world-guard-outcome-{action_id}");
    outcome.idempotency_key = format!("world_guardrail:outcome:{action_id}");
    outcome.evidence_refs.push(format!(
        "parent_guarded_action:{}",
        attempt.parent_action_id
    ));
    if attempt.blocked {
        outcome
            .evidence_refs
            .push("guardrail:tool_run_blocked".into());
    }
    if let Ok(config) = archon_core::config::load_config() {
        super::runtime::record_runtime_guardrail_outcome_for_decision_at_root(
            &config,
            root,
            &decision,
            &attempt.session_id,
            &action_id,
            &format!("{} execution outcome", attempt.tool_name),
            &mut outcome,
        );
    } else if decision.prediction_id.is_some() {
        outcome
            .evidence_refs
            .push("prediction_outcome_unavailable:store_unavailable".into());
    }
    if let Err(error) = archon_world_model::guardrail::append_guardrail_outcome(root, &outcome) {
        tracing::warn!(%error, %action_id, "failed to persist ToolRun outcome");
    }
}

fn record_unavailable_tool_run_outcome(
    root: &Path,
    attempt: &ToolRunAttemptOutcome,
    action_id: &str,
    evidence_ref: &str,
) {
    let mut outcome = unavailable_tool_run_outcome(attempt, action_id);
    outcome.evidence_refs.push(evidence_ref.into());
    if let Err(error) = archon_world_model::guardrail::append_guardrail_outcome(root, &outcome) {
        tracing::warn!(%error, %action_id, "failed to persist unavailable ToolRun outcome");
    }
}

fn unavailable_tool_run_outcome(
    attempt: &ToolRunAttemptOutcome,
    action_id: &str,
) -> archon_world_model::WorldGuardrailOutcome {
    let task_class = archon_world_model::classify_tool_action(
        &attempt.tool_name,
        &attempt.input,
        archon_world_model::integration::WorldAdvisorSurface::ToolRun,
    );
    let mut decision = archon_world_model::WorldGuardrailDecision {
        action_id: action_id.to_string(),
        surface: archon_world_model::integration::WorldAdvisorSurface::ToolRun,
        ..Default::default()
    };
    decision.decision_id.clear();
    let mut outcome = archon_world_model::WorldGuardrailOutcome::from_decision(
        &decision,
        task_class,
        if attempt.blocked || attempt.is_error {
            archon_world_model::GuardrailFinalStatus::Failed
        } else {
            archon_world_model::GuardrailFinalStatus::CompletedWithCaveat
        },
        "ToolRun admission decision unavailable",
    );
    outcome.outcome_id = format!("world-guard-outcome-{action_id}");
    outcome.idempotency_key = format!("world_guardrail:outcome:{action_id}");
    outcome.decision_id = None;
    outcome
}

fn tool_run_action(request: &ToolRunAdmissionRequest) -> archon_world_model::WorldGuardedAction {
    let summary = tool_run_summary(request);
    let mut action = archon_world_model::WorldGuardedAction::new(
        &request.session_id,
        archon_world_model::integration::WorldAdvisorSurface::ToolRun,
        tool_run_action_kind(&request.tool_name),
        &summary,
        &summary,
    );
    action.action_id = tool_run_action_id(request);
    action.parent_action_id = Some(request.parent_action_id.clone());
    action.idempotency_key = format!("world_guardrail:action:{}", action.action_id);
    action
}

fn tool_run_action_kind(tool_name: &str) -> archon_world_model::GuardedActionKind {
    match tool_name {
        "Bash" | "PowerShell" => archon_world_model::GuardedActionKind::ShellCommand,
        "Write" | "Edit" | "ApplyPatch" | "NotebookEdit" => {
            archon_world_model::GuardedActionKind::FileEdit
        }
        _ => archon_world_model::GuardedActionKind::PlanStep,
    }
}

fn tool_run_summary(request: &ToolRunAdmissionRequest) -> String {
    format!(
        "{} invocation, permission={:?}, attempt={}",
        request.tool_name, request.permission_level, request.attempt
    )
}

fn persist_tool_run_candidate_at_root(
    root: &Path,
    request: &ToolRunAdmissionRequest,
) -> anyhow::Result<()> {
    let fields = request
        .input
        .as_object()
        .map(|input| {
            let mut fields = input.keys().map(String::as_str).collect::<Vec<_>>();
            fields.sort_unstable();
            fields.join(",")
        })
        .unwrap_or_default();
    let redacted_input = redact_tool_input(&request.input);
    let mut row = archon_world_model::WorldTraceRow::new(
        &request.session_id,
        archon_world_model::schema::WorldActionKind::ToolCall,
    )
    .with_row_id(tool_run_action_id(request));
    row.redacted_excerpt = Some(format!(
        "tool={} fields={} input={} permission={:?} attempt={}",
        request.tool_name, fields, redacted_input, request.permission_level, request.attempt
    ));
    row.evidence_refs = vec![archon_world_model::schema::EvidenceRef::new(
        "parent_guarded_action",
        &request.parent_action_id,
    )];
    archon_world_model::storage::WorldModelStore::open(root)?.persist_rows(&[row])?;
    Ok(())
}

fn redact_tool_input(input: &serde_json::Value) -> String {
    fn redact(value: &serde_json::Value, key: Option<&str>) -> serde_json::Value {
        if key.is_some_and(is_sensitive_tool_input_key) {
            return serde_json::Value::String("[REDACTED_SECRET]".into());
        }
        match value {
            serde_json::Value::Object(fields) => serde_json::Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| (key.clone(), redact(value, Some(key))))
                    .collect(),
            ),
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.iter().map(|value| redact(value, None)).collect())
            }
            _ => value.clone(),
        }
    }

    archon_world_model::embedding::redact_embedding_text(
        &serde_json::to_string(&redact(input, None)).unwrap_or_default(),
    )
    .chars()
    .take(600)
    .collect()
}

fn is_sensitive_tool_input_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase().replace(['-', '_'], "");
    matches!(
        normalized.as_str(),
        "apikey" | "token" | "secret" | "authorization" | "bearer" | "password"
    )
}

fn tool_run_action_id(request: &ToolRunAdmissionRequest) -> String {
    tool_run_action_id_parts(
        &request.parent_action_id,
        &request.tool_use_id,
        request.attempt,
    )
}

fn tool_run_action_id_parts(parent_action_id: &str, tool_use_id: &str, attempt: u32) -> String {
    format!("world-guard-tool-{parent_action_id}-{tool_use_id}-attempt-{attempt}")
}

fn tool_run_block_reason(decision: &archon_world_model::WorldGuardrailDecision) -> String {
    if decision
        .reason_codes
        .contains(&archon_world_model::GuardrailReasonCode::ToolRunBlocked)
    {
        "critical ToolRun risk in blocking mode".into()
    } else {
        "ToolRun denied by world-model guardrail".into()
    }
}

#[cfg(test)]
fn critical_test_advisory(
    request: &ToolRunAdmissionRequest,
) -> archon_world_model::integration::WorldAdvisorSurfaceRecord {
    let mut prediction = archon_world_model::WorldPrediction::new("test-model", "critical risk");
    prediction.guardrail_scores = Some(archon_world_model::GuardrailRiskScores {
        predicted_failure: Some(1.0),
        ..Default::default()
    });
    archon_world_model::integration::WorldAdvisorSurfaceRecord {
        surface: archon_world_model::integration::WorldAdvisorSurface::ToolRun,
        prediction: Some(prediction),
        unavailable: None,
        session_id: Some(request.session_id.clone()),
        action_ref: Some(tool_run_action_id(request)),
        action_summary: Some(tool_run_summary(request)),
        continue_foreground_flow: true,
        created_at: chrono::Utc::now(),
    }
}

#[cfg(test)]
#[path = "00_tool_run_tests.rs"]
mod tool_run_tests;
