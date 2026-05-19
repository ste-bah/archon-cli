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
fn streaming_text_deltas_coalesce_into_one_line() {
    let mut state = ActivityStreamState::default();
    state.apply_update(update("sherlock", AgentActivityStatus::Running, "HOL"));
    state.apply_update(update("sherlock", AgentActivityStatus::Running, "MES"));
    state.apply_update(update("sherlock", AgentActivityStatus::Running, ": ready"));

    assert_eq!(state.actors.len(), 1);
    assert_eq!(state.actors[0].lines.len(), 1);
    assert_eq!(state.actors[0].lines[0].text, "HOLMES: ready");
}

#[test]
fn streaming_text_deltas_preserve_newlines_inside_coalesced_line() {
    let mut state = ActivityStreamState::default();
    state.apply_update(update("sherlock", AgentActivityStatus::Running, "first"));
    state.apply_update(update("sherlock", AgentActivityStatus::Running, "\nsecond"));

    assert_eq!(state.actors[0].lines.len(), 1);
    assert_eq!(state.actors[0].lines[0].text, "first\nsecond");
}

#[test]
fn activity_text_strips_terminal_escape_sequences() {
    let mut state = ActivityStreamState::default();
    state.apply_update(update(
        "sherlock",
        AgentActivityStatus::Running,
        "\u{1b}[33mhello\u{1b}[0m\rworld\u{0008}!",
    ));

    assert_eq!(state.actors[0].lines[0].text, "hello\nworld!");
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
