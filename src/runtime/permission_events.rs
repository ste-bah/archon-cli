//! Permission runtime event bridge for the governed learning Cozo store.

use std::sync::Arc;

use archon_learning::permission_runtime_events::{
    PermissionRuntimeEventRecord, insert_permission_runtime_event,
};
use cozo::DbInstance;

pub(crate) fn record_permission_event(
    db: Option<&Arc<DbInstance>>,
    session_id: &str,
    agent_type: Option<&str>,
    permission_mode: &str,
    tool_name: &str,
    decision: &str,
    reason: Option<&str>,
) {
    let Some(db) = db else {
        return;
    };
    let event = build_permission_event(
        session_id,
        agent_type,
        permission_mode,
        tool_name,
        decision,
        reason,
    );
    if let Err(error) = insert_permission_runtime_event(db, &event) {
        tracing::warn!(%error, tool = %tool_name, decision, "permission event persistence failed");
    }
}

pub(crate) fn record_permission_mode_event(
    db: Option<&Arc<DbInstance>>,
    session_id: Option<&str>,
    previous_mode: Option<&str>,
    permission_mode: &str,
    decision: &str,
    reason_code: &str,
) {
    let Some(db) = db else {
        return;
    };
    let mut event = PermissionRuntimeEventRecord::new(
        format!("permission-event-{}", uuid::Uuid::new_v4()),
        "PermissionMode",
        permission_mode,
        decision,
        chrono::Utc::now().to_rfc3339(),
    )
    .with_policy_context(Some(reason_code.to_string()), None, None)
    .with_raw_redacted_json(serde_json::json!({
        "source": "permissions_command",
        "previous_mode": previous_mode,
        "permission_mode": permission_mode,
        "payload": "redacted"
    }));
    if let Some(session_id) = session_id.filter(|value| !value.trim().is_empty()) {
        event = event
            .with_session(session_id)
            .with_run_context(Some(session_id.to_string()), None);
    }
    if let Err(error) = insert_permission_runtime_event(db, &event) {
        tracing::warn!(%error, decision, "permission mode event persistence failed");
    }
}

fn build_permission_event(
    session_id: &str,
    agent_type: Option<&str>,
    permission_mode: &str,
    tool_name: &str,
    decision: &str,
    reason: Option<&str>,
) -> PermissionRuntimeEventRecord {
    let reason_code = reason.map(sanitized_permission_reason);
    let mut event = PermissionRuntimeEventRecord::new(
        format!("permission-{}", uuid::Uuid::new_v4()),
        tool_name,
        permission_mode,
        decision,
        chrono::Utc::now().to_rfc3339(),
    )
    .with_session(session_id)
    .with_run_context(
        Some(session_id.to_string()),
        agent_type.map(ToOwned::to_owned),
    )
    .with_policy_context(reason_code.clone(), None, None)
    .with_raw_redacted_json(serde_json::json!({
        "source": "agent_event_forwarder",
        "reason_code": reason_code,
        "payload": "redacted"
    }));
    if decision == "denied" && event.reason_code.is_none() {
        event.reason_code = Some("permission_denied".to_string());
    }
    event
}

fn sanitized_permission_reason(reason: &str) -> String {
    let lower = reason.to_ascii_lowercase();
    if lower.contains("blocked by deny rule") {
        "deny_rule".to_string()
    } else if lower.contains("plan mode") {
        "plan_mode".to_string()
    } else if lower.contains("user_denied_or_timeout") {
        "user_denied_or_timeout".to_string()
    } else if lower.contains("dangerous_operation") {
        "dangerous_operation".to_string()
    } else if lower.contains("risky_operation") {
        "risky_operation".to_string()
    } else if lower.contains("bubble sandbox") {
        "bubble_sandbox".to_string()
    } else if lower.contains("requires confirmation") {
        "rule_requires_confirmation".to_string()
    } else if lower.contains("wants to") {
        "needs_permission".to_string()
    } else {
        "permission_denied".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_permission_event_has_redacted_payload_only() {
        let event = build_permission_event(
            "session-1",
            Some("reviewer"),
            "ask",
            "Bash",
            "denied",
            Some("Blocked by deny rule: tool=Bash, pattern=*"),
        );

        assert_eq!(event.session_id.as_deref(), Some("session-1"));
        assert_eq!(event.run_id.as_deref(), Some("session-1"));
        assert_eq!(event.agent_type.as_deref(), Some("reviewer"));
        assert_eq!(event.tool_name, "Bash");
        assert_eq!(event.permission_mode, "ask");
        assert_eq!(event.decision, "denied");
        assert_eq!(event.reason_code.as_deref(), Some("deny_rule"));
        assert_eq!(event.raw_redacted_json["reason_code"], "deny_rule");
        assert_eq!(event.raw_redacted_json["payload"], "redacted");
        assert!(event.raw_redacted_json.get("reason").is_none());
        assert!(event.raw_redacted_json.get("command").is_none());
        assert!(event.raw_redacted_json.get("file_path").is_none());
    }

    #[test]
    fn permission_reason_sanitizer_does_not_store_raw_details() {
        assert_eq!(
            sanitized_permission_reason("Tool 'Bash' wants to: use Bash"),
            "needs_permission"
        );
        assert_eq!(
            sanitized_permission_reason("Plan mode: tool 'Write' is not allowed"),
            "plan_mode"
        );
        assert_eq!(
            sanitized_permission_reason("/tmp/secret-project command details"),
            "permission_denied"
        );
    }

    #[test]
    fn permission_mode_event_is_session_scoped_and_redacted() {
        let path = format!(
            "/tmp/test-runtime-permission-mode-{}.db",
            uuid::Uuid::new_v4()
        );
        let db = Arc::new(DbInstance::new("sqlite", &path, "").unwrap());
        archon_learning::schema::ensure_learning_schema(&db).unwrap();

        record_permission_mode_event(
            Some(&db),
            Some("session-1"),
            Some("default"),
            "plan",
            "mode_changed",
            "slash_permissions",
        );

        let rows =
            archon_learning::permission_runtime_events::list_permission_runtime_events_by_session(
                &db,
                "session-1",
            )
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].tool_name, "PermissionMode");
        assert_eq!(rows[0].permission_mode, "plan");
        assert_eq!(rows[0].decision, "mode_changed");
        assert_eq!(rows[0].reason_code.as_deref(), Some("slash_permissions"));
        assert_eq!(rows[0].run_id.as_deref(), Some("session-1"));
        assert_eq!(rows[0].raw_redacted_json["previous_mode"], "default");
        assert_eq!(rows[0].raw_redacted_json["payload"], "redacted");
    }
}
