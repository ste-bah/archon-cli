use super::*;

#[test]
fn variant_name_labels_events_used_in_drain_forensics() {
    assert_eq!(
        TuiEvent::TextDelta("hello".into()).variant_name(),
        "TextDelta"
    );
    assert_eq!(
        TuiEvent::SessionRenamed("session".into()).variant_name(),
        "SessionRenamed"
    );
    assert_eq!(
        TuiEvent::AgentActivity(AgentActivityUpdate {
            id: "agent-1".into(),
            name: "Agent".into(),
            role: AgentActivityRole::Subagent,
            status: AgentActivityStatus::Running,
            current_tool: None,
            detail: None,
            run_id: None,
            parent_id: None,
            artifact_id: None,
            provider: Some("openai-codex".into()),
            model: Some("gpt-5.4".into()),
            cost_usd: Some(0.01),
        })
        .variant_name(),
        "AgentActivity"
    );
}

#[test]
fn agent_activity_update_uses_subagent_type_as_name() {
    let update = AgentActivityUpdate::from(
        archon_observability::AgentActivityEvent::new(
            "session-1",
            archon_observability::AgentActivityKind::AgentSpawned,
            archon_observability::AgentActivityStatus::Running,
            "running",
        )
        .with_subagent_id("subagent-1")
        .with_subagent_type("sherlock-holmes"),
    );

    assert_eq!(update.id, "subagent-1");
    assert_eq!(update.name, "sherlock-holmes");
    assert_eq!(update.role, AgentActivityRole::Subagent);
}

#[test]
fn activity_stream_update_parses_internal_payload() {
    let payload = format!(
        "{}{}",
        ACTIVITY_STREAM_PREFIX,
        serde_json::json!({
            "kind": "text",
            "text": "working",
            "tool": null,
            "is_error": false
        })
    );
    let update = ActivityStreamUpdate::from_activity_event(
        archon_observability::AgentActivityEvent::new(
            "session-1",
            archon_observability::AgentActivityKind::AgentRunning,
            archon_observability::AgentActivityStatus::Running,
            payload,
        )
        .with_subagent_id("agent-1")
        .with_subagent_type("sherlock-holmes"),
    );

    assert_eq!(update.id, "agent-1");
    assert_eq!(update.name, "sherlock-holmes");
    assert_eq!(update.kind, ActivityStreamLineKind::Text);
    assert_eq!(update.text, "working");
}
