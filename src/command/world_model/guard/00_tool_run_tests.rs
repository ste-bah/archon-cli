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

    fn capability(&self) -> archon_tools::tool::ToolCapability {
        archon_tools::tool::ToolCapability::EXECUTION
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
        admission_evaluated: true,
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
        admission_evaluated: true,
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
        admission_evaluated: true,
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
    let decisions = archon_world_model::guardrail::load_guardrail_decisions(temp.path()).unwrap();
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
    let decisions = archon_world_model::guardrail::load_guardrail_decisions(temp.path()).unwrap();
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
