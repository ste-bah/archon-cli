use std::path::Path;

use archon_tools::tool::{ToolRunAdmission, ToolRunAdmissionRequest, ToolRunAttemptOutcome};

pub(crate) fn admit_tool_run_attempt(
    config: &archon_core::config::ArchonConfig,
    request: ToolRunAdmissionRequest,
) -> ToolRunAdmission {
    let Ok(root) = super::world_model_root() else {
        return ToolRunAdmission::Allowed;
    };
    let policy = policy_from_config(config);
    let mode = archon_world_model::guardrail::mode_for_surface(
        &policy,
        archon_world_model::integration::WorldAdvisorSurface::ToolRun,
    );
    if !policy.enabled || matches!(mode, archon_world_model::WorldGuardrailMode::Off) {
        return ToolRunAdmission::Allowed;
    }

    let started_at = std::time::Instant::now();
    if let Err(error) = persist_tool_run_candidate_at_root(&root, &request) {
        tracing::warn!(%error, "failed to persist ToolRun candidate trace; allowing attempt");
        return ToolRunAdmission::Allowed;
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
    admit_tool_run_with_policy_at_root(&root, &request, policy, mode, advisory, started_at)
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
    if let Err(error) = archon_world_model::guardrail::append_guardrail_revision(
        root,
        action,
        decision.clone(),
        revision_key,
    ) {
        tracing::warn!(%error, "failed to persist ToolRun admission; allowing attempt");
        return ToolRunAdmission::Allowed;
    }

    if decision.allowed_to_continue {
        ToolRunAdmission::Allowed
    } else {
        ToolRunAdmission::Blocked {
            reason: tool_run_block_reason(&decision),
        }
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
    let mut decision = archon_world_model::WorldGuardrailDecision::default();
    decision.action_id = action_id.to_string();
    decision.surface = archon_world_model::integration::WorldAdvisorSurface::ToolRun;
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
mod tool_run_tests {
    use super::*;
    use archon_tools::tool::{PermissionLevel, Tool, ToolContext, ToolResult};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct BlockingTestTool {
        executions: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Tool for BlockingTestTool {
        fn name(&self) -> &str {
            "Bash"
        }

        fn description(&self) -> &str {
            "blocked integration test"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            self.executions.fetch_add(1, Ordering::SeqCst);
            ToolResult::success("executed")
        }

        fn permission_level(&self, _input: &serde_json::Value) -> PermissionLevel {
            PermissionLevel::Dangerous
        }
    }

    #[test]
    fn tool_run_attempt_identity_is_stable_per_attempt_and_unique_across_retries() {
        let request = ToolRunAdmissionRequest {
            session_id: "session-1".into(),
            parent_action_id: "parent-1".into(),
            tool_use_id: "tool-use-1".into(),
            attempt: 0,
            tool_name: "Edit".into(),
            input: serde_json::json!({"file_path": "src/lib.rs"}),
            permission_level: archon_tools::tool::PermissionLevel::Risky,
        };
        let mut retry = request.clone();
        retry.attempt = 1;

        assert_eq!(tool_run_action_id(&request), tool_run_action_id(&request));
        assert_ne!(tool_run_action_id(&request), tool_run_action_id(&retry));
        assert!(
            tool_run_action(&retry)
                .idempotency_key
                .ends_with("attempt-1")
        );
    }

    #[test]
    fn tool_run_candidate_trace_uses_real_action_id_and_redacted_input_shape() {
        let temp = tempfile::tempdir().unwrap();
        let request = ToolRunAdmissionRequest {
            session_id: "session-1".into(),
            parent_action_id: "parent-1".into(),
            tool_use_id: "tool-use-1".into(),
            attempt: 2,
            tool_name: "Bash".into(),
            input: serde_json::json!({
                "command": "curl -H 'Authorization: secret-token'",
                "api_key": "direct-secret",
                "nested": {
                    "token": "nested-secret",
                    "items": [{"authorization": "array-secret"}]
                }
            }),
            permission_level: archon_tools::tool::PermissionLevel::Dangerous,
        };

        persist_tool_run_candidate_at_root(temp.path(), &request).unwrap();

        let rows = archon_world_model::storage::WorldModelStore::open(temp.path())
            .unwrap()
            .load_rows()
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].row_id, tool_run_action_id(&request));
        assert_eq!(
            rows[0].action_kind,
            archon_world_model::schema::WorldActionKind::ToolCall
        );
        let excerpt = rows[0].redacted_excerpt.as_deref().unwrap();
        assert!(excerpt.contains("tool=Bash"));
        assert!(excerpt.contains("command"));
        assert!(!excerpt.contains("secret-token"));
        assert!(!excerpt.contains("direct-secret"));
        assert!(!excerpt.contains("nested-secret"));
        assert!(!excerpt.contains("array-secret"));
    }

    #[test]
    fn blocked_attempt_records_exactly_one_correlated_outcome() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = archon_core::config::ArchonConfig::default();
        config.learning.world_model.guardrails.tool_run_mode = "guarded".into();
        let request = ToolRunAdmissionRequest {
            session_id: "session-1".into(),
            parent_action_id: "parent-1".into(),
            tool_use_id: "tool-use-1".into(),
            attempt: 3,
            tool_name: "Bash".into(),
            input: serde_json::json!({"command": "dangerous command"}),
            permission_level: archon_tools::tool::PermissionLevel::Dangerous,
        };
        let _ = admit_tool_run_at_root(
            &config,
            temp.path(),
            &request,
            critical_test_advisory(&request),
        );
        let outcome = ToolRunAttemptOutcome {
            session_id: request.session_id.clone(),
            parent_action_id: request.parent_action_id.clone(),
            tool_use_id: request.tool_use_id.clone(),
            attempt: request.attempt,
            tool_name: request.tool_name.clone(),
            input: request.input.clone(),
            permission_level: request.permission_level,
            blocked: true,
            is_error: true,
        };

        record_tool_run_attempt_outcome_at_root(temp.path(), outcome.clone());
        record_tool_run_attempt_outcome_at_root(temp.path(), outcome);

        let outcomes = archon_world_model::guardrail::load_guardrail_outcomes(temp.path()).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].action_id, tool_run_action_id(&request));
        assert!(
            outcomes[0]
                .evidence_refs
                .contains(&"prediction_outcome_unavailable:store_unavailable".into())
        );
        assert_eq!(
            outcomes[0].decision_id.as_deref(),
            archon_world_model::guardrail::load_guardrail_decisions(temp.path())
                .unwrap()
                .first()
                .map(|decision| decision.decision_id.as_str())
        );
    }

    #[test]
    fn decision_load_failure_records_correlated_unavailable_outcome() {
        let temp = tempfile::tempdir().unwrap();
        let attempt = ToolRunAttemptOutcome {
            session_id: "session-1".into(),
            parent_action_id: "parent-1".into(),
            tool_use_id: "tool-use-1".into(),
            attempt: 4,
            tool_name: "Bash".into(),
            input: serde_json::json!({"command": "true"}),
            permission_level: archon_tools::tool::PermissionLevel::Dangerous,
            blocked: false,
            is_error: false,
        };
        let action_id = tool_run_action_id_parts(
            &attempt.parent_action_id,
            &attempt.tool_use_id,
            attempt.attempt,
        );
        let ledger_dir = temp.path().join("ledgers");
        std::fs::create_dir_all(&ledger_dir).unwrap();
        std::fs::write(
            ledger_dir.join(archon_world_model::guardrail::REVISIONS_LEDGER),
            b"not-json\n",
        )
        .unwrap();

        record_tool_run_attempt_outcome_at_root(temp.path(), attempt);

        let outcomes = archon_world_model::guardrail::load_guardrail_outcomes(temp.path()).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].action_id, action_id);
        assert_eq!(outcomes[0].decision_id, None);
        assert!(
            outcomes[0]
                .evidence_refs
                .contains(&"guardrail_decision_unavailable:store_unavailable".into())
        );
    }

    #[test]
    fn missing_decision_records_correlated_unavailable_outcome() {
        let temp = tempfile::tempdir().unwrap();
        let attempt = ToolRunAttemptOutcome {
            session_id: "session-1".into(),
            parent_action_id: "parent-1".into(),
            tool_use_id: "tool-use-1".into(),
            attempt: 5,
            tool_name: "Bash".into(),
            input: serde_json::json!({"command": "true"}),
            permission_level: archon_tools::tool::PermissionLevel::Dangerous,
            blocked: false,
            is_error: false,
        };
        let action_id = tool_run_action_id_parts(
            &attempt.parent_action_id,
            &attempt.tool_use_id,
            attempt.attempt,
        );

        record_tool_run_attempt_outcome_at_root(temp.path(), attempt);

        let outcomes = archon_world_model::guardrail::load_guardrail_outcomes(temp.path()).unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].action_id, action_id);
        assert_eq!(outcomes[0].decision_id, None);
        assert!(
            outcomes[0]
                .evidence_refs
                .contains(&"guardrail_decision_unavailable:not_found".into())
        );
    }

    #[tokio::test]
    async fn blocked_dispatch_persists_decision_before_skipping_execution_and_records_outcome() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = archon_core::config::ArchonConfig::default();
        config.learning.world_model.guardrails.tool_run_mode = "guarded".into();
        let executions = Arc::new(AtomicUsize::new(0));
        let mut registry = archon_core::dispatch::ToolRegistry::new();
        registry.register(Box::new(BlockingTestTool {
            executions: Arc::clone(&executions),
        }));
        let admission_config = config.clone();
        let admission_root = temp.path().to_path_buf();
        let outcome_root = temp.path().to_path_buf();
        let ctx = ToolContext {
            session_id: "session-1".into(),
            tool_run_parent_action_id: Some("parent-1".into()),
            tool_run_tool_use_id: Some("tool-use-1".into()),
            tool_run_attempt: 0,
            tool_run_admission: Some(Arc::new(move |request| {
                let advisory = critical_test_advisory(&request);
                admit_tool_run_at_root(&admission_config, &admission_root, &request, advisory)
            })),
            tool_run_outcome: Some(Arc::new(move |outcome| {
                record_tool_run_attempt_outcome_at_root(&outcome_root, outcome);
            })),
            ..ToolContext::default()
        };

        let result = registry
            .dispatch(
                "Bash",
                serde_json::json!({"command": "dangerous command"}),
                &ctx,
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("critical ToolRun risk"));
        assert_eq!(executions.load(Ordering::SeqCst), 0);
        let actions = archon_world_model::guardrail::load_guarded_actions(temp.path()).unwrap();
        let decisions =
            archon_world_model::guardrail::load_guardrail_decisions(temp.path()).unwrap();
        let outcomes = archon_world_model::guardrail::load_guardrail_outcomes(temp.path()).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].parent_action_id.as_deref(), Some("parent-1"));
        assert_eq!(decisions.len(), 1);
        assert!(!decisions[0].allowed_to_continue);
        assert!(
            decisions[0]
                .reason_codes
                .contains(&archon_world_model::GuardrailReasonCode::ToolRunBlocked)
        );
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].action_id, actions[0].action_id);
        assert_eq!(
            outcomes[0].decision_id.as_deref(),
            Some(decisions[0].decision_id.as_str())
        );
    }

    #[test]
    fn critical_tool_run_persists_blocking_child_before_returning_blocked() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = archon_core::config::ArchonConfig::default();
        config.learning.world_model.guardrails.tool_run_mode = "guarded".into();
        let request = ToolRunAdmissionRequest {
            session_id: "session-1".into(),
            parent_action_id: "parent-1".into(),
            tool_use_id: "tool-use-1".into(),
            attempt: 3,
            tool_name: "Bash".into(),
            input: serde_json::json!({"command": "dangerous command"}),
            permission_level: archon_tools::tool::PermissionLevel::Dangerous,
        };

        let verdict = admit_tool_run_at_root(
            &config,
            temp.path(),
            &request,
            critical_test_advisory(&request),
        );

        assert!(matches!(verdict, ToolRunAdmission::Blocked { .. }));
        let actions = archon_world_model::guardrail::load_guarded_actions(temp.path()).unwrap();
        let decisions =
            archon_world_model::guardrail::load_guardrail_decisions(temp.path()).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].parent_action_id.as_deref(), Some("parent-1"));
        assert!(actions[0].action_id.ends_with("tool-use-1-attempt-3"));
        assert_eq!(decisions.len(), 1);
        assert!(!decisions[0].allowed_to_continue);
        assert!(
            decisions[0]
                .reason_codes
                .contains(&archon_world_model::GuardrailReasonCode::ToolRunBlocked)
        );
    }
}
