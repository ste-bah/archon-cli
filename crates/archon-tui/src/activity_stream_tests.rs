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

#[test]
fn open_follows_latest_actor_updates() {
    let mut state = ActivityStreamState::default();
    state.apply_update(update("parent", AgentActivityStatus::Running, "one"));
    state.apply_update(update("sherlock", AgentActivityStatus::Running, "two"));
    state.open();
    assert_eq!(state.actors[state.selected].id, "sherlock");
    state.apply_update(update("critic", AgentActivityStatus::Running, "three"));
    assert_eq!(state.actors[state.selected].id, "critic");
}

#[test]
fn manual_scroll_preserves_selected_actor_until_follow_resumes() {
    let mut state = ActivityStreamState::default();
    state.apply_update(update("sherlock", AgentActivityStatus::Running, "one"));
    state.apply_update(update("critic", AgentActivityStatus::Running, "two"));
    state.open();
    state.select_next();
    assert_eq!(state.actors[state.selected].id, "sherlock");
    state.apply_update(update("critic", AgentActivityStatus::Running, "three"));
    assert_eq!(state.actors[state.selected].id, "sherlock");
    state.scroll_bottom();
    state.apply_update(update("critic", AgentActivityStatus::Running, "four"));
    assert_eq!(state.actors[state.selected].id, "critic");
}
