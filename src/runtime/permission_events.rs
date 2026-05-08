//! Permission runtime event bridge for the governed learning Cozo store.

use std::sync::Arc;

use archon_learning::permission_runtime_events::{
    PermissionRuntimeEventRecord, insert_permission_runtime_event,
};
use cozo::DbInstance;

pub(crate) fn record_permission_event(
    db: Option<&Arc<DbInstance>>,
    session_id: &str,
    permission_mode: &str,
    tool_name: &str,
    decision: &str,
) {
    let Some(db) = db else {
        return;
    };
    let event = build_permission_event(session_id, permission_mode, tool_name, decision);
    if let Err(error) = insert_permission_runtime_event(db, &event) {
        tracing::warn!(%error, tool = %tool_name, decision, "permission event persistence failed");
    }
}

fn build_permission_event(
    session_id: &str,
    permission_mode: &str,
    tool_name: &str,
    decision: &str,
) -> PermissionRuntimeEventRecord {
    PermissionRuntimeEventRecord::new(
        format!("permission-{}", uuid::Uuid::new_v4()),
        tool_name,
        permission_mode,
        decision,
        chrono::Utc::now().to_rfc3339(),
    )
    .with_session(session_id)
    .with_raw_redacted_json(serde_json::json!({
        "source": "agent_event_forwarder",
        "payload": "redacted"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_permission_event_has_redacted_payload_only() {
        let event = build_permission_event("session-1", "ask", "Bash", "denied");

        assert_eq!(event.session_id.as_deref(), Some("session-1"));
        assert_eq!(event.tool_name, "Bash");
        assert_eq!(event.permission_mode, "ask");
        assert_eq!(event.decision, "denied");
        assert_eq!(event.raw_redacted_json["payload"], "redacted");
        assert!(event.raw_redacted_json.get("command").is_none());
        assert!(event.raw_redacted_json.get("file_path").is_none());
    }
}
