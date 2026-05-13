use super::*;

fn update(id: &str, status: AgentActivityStatus, text: &str) -> ActivityStreamUpdate {
    ActivityStreamUpdate {
        id: id.into(),
        name: id.into(),
        role: AgentActivityRole::Subagent,
        status,
        provider: Some("anthropic".into()),
        model: Some("claude-sonnet-4-6".into()),
        kind: ActivityStreamLineKind::Text,
        text: text.into(),
        tool: None,
        is_error: false,
    }
}

#[test]
fn stream_keeps_actor_lines() {
    let mut state = ActivityStreamState::default();
    state.apply_update(update("sherlock", AgentActivityStatus::Running, "digging"));
    assert_eq!(state.actors.len(), 1);
    assert_eq!(state.actors[0].lines[0].text, "digging");
}

#[test]
fn foreground_can_be_backgrounded() {
    let mut state = ActivityStreamState::default();
    state.open();
    assert!(state.is_foreground());
    state.background();
    assert!(!state.is_foreground());
}
