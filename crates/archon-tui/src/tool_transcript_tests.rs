use crate::app::App;
use crate::input::{KeyResult, handle_key};
use crate::keybindings::KeyMap;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn complete_success(app: &mut App, name: &str, id: &str, output: &str) {
    app.on_tool_start(name, id);
    app.on_tool_complete(name, id, true, output);
}

#[test]
fn successful_tool_completion_appends_collapsed_transcript_marker() {
    let mut app = App::new();
    complete_success(&mut app, "Read", "read-1", "first\nsecond");

    let line = app.output.all_lines().join("\n");
    assert!(line.contains("● Read ✓"));
    assert!(line.contains("(2 lines)"));
}

#[test]
fn successful_completion_without_summary_uses_name_only_marker() {
    let mut app = App::new();
    complete_success(&mut app, "Read", "read-1", "contents");

    let line = app.output.all_lines().join("\n");
    assert!(line.contains("● Read ✓"));
    assert!(!line.contains("Read("));
}

#[test]
fn successful_empty_output_reports_zero_lines() {
    let mut app = App::new();
    complete_success(&mut app, "Read", "read-1", "");

    assert!(app.output.all_lines().join("\n").contains("(0 lines)"));
}

#[test]
fn expanded_tool_output_is_bounded_utf8_safe_and_collapses_cleanly() {
    let mut app = App::new();
    let output = format!("{}\nsecond\nthird\nfourth", "é".repeat(700));
    complete_success(&mut app, "Read", "read-1", &output);
    let collapsed = app
        .output
        .all_lines()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    app.toggle_tool_output(None);
    let expanded = app.output.all_lines();
    assert_eq!(expanded[0], collapsed[0]);
    let excerpt = expanded[1..].join("\n");
    assert!(excerpt.contains('…'));
    assert!(excerpt.chars().count() <= 600);
    assert!(
        expanded.len() <= 4,
        "marker plus at most three excerpt lines"
    );

    app.toggle_tool_output(None);
    assert_eq!(app.output.all_lines(), collapsed);
}

#[test]
fn overlapping_same_name_completions_match_by_tool_id() {
    let mut app = App::new();
    app.on_tool_start("Bash", "bash-1");
    app.on_tool_start("Bash", "bash-2");
    app.on_tool_complete("Bash", "bash-1", true, "first");

    assert_eq!(app.tool_outputs[0].output, "first");
    assert!(app.tool_outputs[1].output.is_empty());
    app.on_tool_complete("Bash", "bash-2", true, "second");
    assert_eq!(app.tool_outputs[1].output, "second");
}

#[test]
fn tool_expansion_shifts_later_thinking_marker_and_collapse_restores_it() {
    let mut app = App::new();
    complete_success(&mut app, "Read", "read-1", "one\ntwo");
    app.on_thinking_delta("thought");
    app.on_turn_complete();

    app.toggle_tool_output(None);
    app.toggle_thinking();
    let expanded = app.output.all_lines();
    let thought = expanded
        .iter()
        .position(|line| line.contains("Thought for"))
        .expect("thinking marker");
    assert_eq!(expanded[thought + 1], "  thought");

    app.toggle_thinking();
    app.toggle_tool_output(None);
    assert_eq!(app.output.all_lines().len(), 3);
}

#[test]
fn failed_tool_keeps_full_inline_failure_without_success_marker() {
    let mut app = App::new();
    complete_success(&mut app, "Read", "read-ok", "ok");
    app.on_tool_start("Bash", "bash-fail");
    app.on_tool_complete("Bash", "bash-fail", false, "full failure detail");

    let transcript = app.output.all_lines().join("\n");
    assert!(transcript.contains("[tool] Bash failed:\nfull failure detail"));
    assert_eq!(transcript.matches("● Bash").count(), 0);
}

#[test]
fn thinking_expansion_shifts_later_tool_marker() {
    let mut app = App::new();
    complete_success(&mut app, "Read", "read-1", "first");
    app.on_thinking_delta("thought");
    app.on_turn_complete();
    complete_success(&mut app, "Read", "read-2", "second");

    app.toggle_thinking();
    app.toggle_tool_output(Some(1));

    let lines = app.output.all_lines();
    let second_marker = lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.contains("● Read ✓") && line.contains("(1 lines)"))
        .nth(1)
        .map(|(index, _)| index)
        .expect("second tool marker");
    assert_eq!(lines[second_marker + 1], "second");
}

#[test]
fn completion_without_matching_start_is_safe() {
    let mut app = App::new();
    app.on_tool_complete("Read", "missing", true, "output");

    assert!(app.output.all_lines().is_empty());
    assert!(app.tool_outputs.is_empty());
}

#[test]
fn ctrl_e_toggles_latest_tool_output_without_replacing_ctrl_t_thinking_toggle() {
    let mut app = App::new();
    complete_success(&mut app, "Read", "read-1", "one");
    let keymap = KeyMap::default();

    assert!(matches!(
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
            &keymap,
        ),
        KeyResult::Nothing
    ));
    assert!(app.tool_outputs[0].expanded);

    app.on_thinking_delta("thought");
    app.on_turn_complete();
    assert!(matches!(
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL),
            &keymap,
        ),
        KeyResult::Nothing
    ));
    assert!(app.thinking_blocks[0].expanded);
}
