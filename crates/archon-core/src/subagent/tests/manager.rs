use super::*;

#[test]
fn register_returns_uuid() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).expect("should register");

    // UUID v4 format: 8-4-4-4-12 hex chars
    assert_eq!(id.len(), 36);
    assert!(id.contains('-'));
}

#[test]
fn get_status_returns_running() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();

    let info = mgr.get_status(&id).expect("should exist");
    assert_eq!(info.status, SubagentStatus::Running);
    assert_eq!(info.request.prompt, "Analyze the codebase");
    assert!(info.result.is_none());
}

#[test]
fn list_active_only_returns_running() {
    let mut mgr = SubagentManager::default();
    let id1 = mgr.register(sample_request()).unwrap();
    let _id2 = mgr.register(sample_request()).unwrap();

    assert_eq!(mgr.list_active().len(), 2);

    mgr.complete(&id1, "done".into()).unwrap();
    assert_eq!(mgr.list_active().len(), 1);
}

#[test]
fn complete_sets_result() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();

    mgr.complete(&id, "task finished successfully".into())
        .unwrap();

    let info = mgr.get_status(&id).unwrap();
    assert_eq!(info.status, SubagentStatus::Completed);
    assert_eq!(info.result.as_deref(), Some("task finished successfully"));
}

#[test]
fn complete_nonexistent_returns_error() {
    let mut mgr = SubagentManager::default();
    let err = mgr.complete("fake-id", "result".into()).unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn complete_already_completed_returns_error() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();
    mgr.complete(&id, "first".into()).unwrap();

    let err = mgr.complete(&id, "second".into()).unwrap_err();
    assert!(err.to_string().contains("not in Running state"));
}

#[test]
fn max_concurrent_enforced() {
    let mut mgr = SubagentManager::new(2);
    mgr.register(sample_request()).unwrap();
    mgr.register(sample_request()).unwrap();

    let err = mgr.register(sample_request()).unwrap_err();
    assert!(err.to_string().contains("max concurrent"));
}

#[test]
fn mark_timed_out_works() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();

    mgr.mark_timed_out(&id).unwrap();
    assert_eq!(
        mgr.get_status(&id).unwrap().status,
        SubagentStatus::TimedOut
    );
}

#[test]
fn mark_failed_works() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();

    mgr.mark_failed(&id, "something went wrong".into()).unwrap();
    assert_eq!(
        mgr.get_status(&id).unwrap().status,
        SubagentStatus::Failed("something went wrong".into())
    );
}

#[test]
fn get_status_nonexistent_returns_none() {
    let mgr = SubagentManager::default();
    assert!(mgr.get_status("nonexistent").is_none());
}

// -----------------------------------------------------------------------
// Auto-background tests (AGT-025)
// -----------------------------------------------------------------------

#[test]
fn auto_background_constant_is_120s() {
    assert_eq!(AUTO_BACKGROUND_MS, 120_000);
}

#[test]
fn auto_background_disabled_by_default() {
    unsafe {
        std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
    }
    assert!(!is_auto_background_enabled());
    assert_eq!(get_auto_background_ms(), 0);
}

#[test]
fn auto_background_enabled_with_1() {
    unsafe {
        std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", "1");
    }
    assert!(is_auto_background_enabled());
    assert_eq!(get_auto_background_ms(), 120_000);
    unsafe {
        std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
    }
}

#[test]
fn auto_background_enabled_with_true() {
    unsafe {
        std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", "true");
    }
    assert!(is_auto_background_enabled());
    unsafe {
        std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
    }
}

#[test]
fn auto_background_disabled_for_zero() {
    unsafe {
        std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", "0");
    }
    assert!(!is_auto_background_enabled());
    assert_eq!(get_auto_background_ms(), 0);
    unsafe {
        std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
    }
}

#[test]
fn auto_background_case_insensitive() {
    unsafe {
        std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", "TRUE");
    }
    assert!(is_auto_background_enabled());
    unsafe {
        std::env::set_var("ARCHON_AUTO_BACKGROUND_TASKS", "True");
    }
    assert!(is_auto_background_enabled());
    unsafe {
        std::env::remove_var("ARCHON_AUTO_BACKGROUND_TASKS");
    }
}

// -----------------------------------------------------------------------
// Name registry tests (AGT-026)
// -----------------------------------------------------------------------

#[test]
fn register_name_and_resolve() {
    let mut mgr = SubagentManager::default();
    mgr.register_name("explorer".into(), "agent-uuid-123".into());

    assert_eq!(mgr.resolve_name("explorer"), Some("agent-uuid-123"));
    assert_eq!(mgr.resolve_name("unknown"), None);
}

#[test]
fn unregister_name_removes_entry() {
    let mut mgr = SubagentManager::default();
    mgr.register_name("explorer".into(), "agent-uuid-123".into());
    mgr.unregister_name("explorer");

    assert_eq!(mgr.resolve_name("explorer"), None);
}

#[test]
fn register_name_overwrites_previous() {
    let mut mgr = SubagentManager::default();
    mgr.register_name("explorer".into(), "old-id".into());
    mgr.register_name("explorer".into(), "new-id".into());

    assert_eq!(mgr.resolve_name("explorer"), Some("new-id"));
}

#[test]
fn is_running_checks_status() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();

    assert!(mgr.is_running(&id));

    mgr.complete(&id, "done".into()).unwrap();
    assert!(!mgr.is_running(&id));
}

#[test]
fn is_running_false_for_nonexistent() {
    let mgr = SubagentManager::default();
    assert!(!mgr.is_running("nonexistent-id"));
}

#[test]
fn has_agent_checks_existence() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();

    assert!(mgr.has_agent(&id));
    assert!(!mgr.has_agent("nonexistent-id"));

    // Completed agents still exist in state
    mgr.complete(&id, "done".into()).unwrap();
    assert!(mgr.has_agent(&id));
}

// -----------------------------------------------------------------------
// Pending message tests (AGT-026)
// -----------------------------------------------------------------------

#[test]
fn queue_and_drain_pending_messages() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();

    mgr.queue_pending_message(&id, "msg1".into());
    mgr.queue_pending_message(&id, "msg2".into());
    mgr.queue_pending_message(&id, "msg3".into());

    let drained = mgr.drain_pending_messages(&id);
    assert_eq!(drained, vec!["msg1", "msg2", "msg3"]);

    // Second drain returns empty (queue was cleared)
    let drained2 = mgr.drain_pending_messages(&id);
    assert!(drained2.is_empty());
}

#[test]
fn drain_nonexistent_agent_returns_empty() {
    let mut mgr = SubagentManager::default();
    let drained = mgr.drain_pending_messages("nonexistent-id");
    assert!(drained.is_empty());
}

#[test]
fn pending_messages_are_fifo() {
    let mut mgr = SubagentManager::default();
    mgr.queue_pending_message("agent-1", "first".into());
    mgr.queue_pending_message("agent-1", "second".into());
    mgr.queue_pending_message("agent-1", "third".into());

    let drained = mgr.drain_pending_messages("agent-1");
    assert_eq!(drained[0], "first");
    assert_eq!(drained[1], "second");
    assert_eq!(drained[2], "third");
}

#[test]
fn pending_messages_isolated_per_agent() {
    let mut mgr = SubagentManager::default();
    mgr.queue_pending_message("agent-1", "msg-a".into());
    mgr.queue_pending_message("agent-2", "msg-b".into());

    let drained1 = mgr.drain_pending_messages("agent-1");
    assert_eq!(drained1, vec!["msg-a"]);

    let drained2 = mgr.drain_pending_messages("agent-2");
    assert_eq!(drained2, vec!["msg-b"]);
}

// -----------------------------------------------------------------------
// Cleanup tests (AGT-026)
// -----------------------------------------------------------------------

#[test]
fn cleanup_agent_drops_pending_messages() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();
    mgr.queue_pending_message(&id, "lost message".into());

    mgr.cleanup_agent(&id);

    let drained = mgr.drain_pending_messages(&id);
    assert!(
        drained.is_empty(),
        "pending messages should be lost on cleanup"
    );
}

#[test]
fn cleanup_agent_removes_name_registry_entry() {
    let mut mgr = SubagentManager::default();
    let id = mgr.register(sample_request()).unwrap();
    mgr.register_name("explorer".into(), id.clone());

    mgr.cleanup_agent(&id);

    assert_eq!(
        mgr.resolve_name("explorer"),
        None,
        "name should be removed on cleanup"
    );
}

#[test]
fn cleanup_only_removes_matching_name() {
    let mut mgr = SubagentManager::default();
    let id1 = mgr.register(sample_request()).unwrap();
    let id2 = mgr.register(sample_request()).unwrap();
    mgr.register_name("explorer".into(), id1.clone());
    mgr.register_name("reviewer".into(), id2.clone());

    mgr.cleanup_agent(&id1);

    assert_eq!(mgr.resolve_name("explorer"), None);
    assert_eq!(mgr.resolve_name("reviewer"), Some(id2.as_str()));
}

// -----------------------------------------------------------------------
// TASK-T2 (G2): Structured envelope delivery via queue
// -----------------------------------------------------------------------

#[test]
fn structured_envelope_delivers_through_queue() {
    use archon_tools::send_message::{SendMessageRequest, build_structured_envelope};

    let mut mgr = SubagentManager::new(4);
    let id_a = mgr.register(sample_request()).unwrap();
    let id_b = mgr.register(sample_request()).unwrap();

    let envelope_req = SendMessageRequest {
        to: id_b.clone(),
        message: String::new(),
        summary: None,
        message_type: "shutdown_response".into(),
        request_id: Some("req-1".into()),
        approve: Some(true),
        reason: Some("done".into()),
        feedback: None,
    };
    let envelope = build_structured_envelope(&envelope_req);
    mgr.queue_pending_message(&id_b, envelope);

    let drained = mgr.drain_pending_messages(&id_b);
    assert_eq!(drained.len(), 1);
    assert!(
        drained[0].starts_with("<archon_structured_message type=\"shutdown_response\""),
        "envelope should start with structured opening tag: {}",
        drained[0]
    );
    assert!(drained[0].contains("request_id=\"req-1\""));
    assert!(drained[0].contains("approve=\"true\""));
    assert!(drained[0].contains("<reason>done</reason>"));
    assert!(drained[0].ends_with("</archon_structured_message>"));

    // Agent A's queue should remain untouched
    assert!(mgr.drain_pending_messages(&id_a).is_empty());
}
