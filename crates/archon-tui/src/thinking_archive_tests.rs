use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::app::App;
use super::output::ThinkingState;

#[test]
fn completed_empty_thinking_is_still_archived() {
    let mut app = App::new();

    app.on_thinking_delta("");
    app.on_turn_complete();

    assert_eq!(app.thinking_blocks.len(), 1);
    assert!(app.thinking_blocks[0].text.is_empty());
    assert!(app.output.all_lines()[0].starts_with("✻ Thought for "));
}

#[test]
fn hidden_thinking_is_captured_and_archived_on_completion() {
    let mut app = App::new();
    assert!(!app.show_thinking);

    app.on_thinking_delta("hidden reasoning");
    app.on_turn_complete();

    assert_eq!(app.thinking_blocks.len(), 1);
    assert_eq!(app.thinking_blocks[0].text, "hidden reasoning");
    assert!(
        app.output
            .all_lines()
            .iter()
            .any(|line| line.contains("Thought for"))
    );
}

#[test]
fn consecutive_turns_archive_distinct_thinking_blocks() {
    let mut app = App::new();

    app.on_thinking_delta("first");
    app.on_turn_complete();
    app.on_thinking_delta("second");
    app.on_turn_complete();

    let texts = app
        .thinking_blocks
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(texts, ["first", "second"]);
}

#[test]
fn thinking_capture_retains_only_the_bounded_utf8_tail() {
    let mut thinking = ThinkingState::new();
    let oversized = format!(
        "{}{}",
        "x".repeat(ThinkingState::MAX_CAPTURE_BYTES),
        "界tail"
    );

    thinking.on_thinking_delta(&oversized);

    assert!(thinking.accumulated.len() <= ThinkingState::MAX_CAPTURE_BYTES);
    assert!(thinking.accumulated.ends_with("界tail"));
    assert!(thinking.accumulated.is_char_boundary(0));
}

#[test]
fn collapsed_active_thinking_includes_a_tail_preview() {
    let mut app = App::new();
    app.show_thinking = true;
    app.on_thinking_delta("first line\nlast thought");

    let rendered = app
        .thinking_lines(80)
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    assert_eq!(rendered.len(), 2);
    assert!(rendered[1].contains("last thought"));
}

#[test]
fn active_preview_is_one_display_row_and_uses_the_newest_wrapped_suffix() {
    let mut app = App::new();
    app.show_thinking = true;
    app.on_thinking_delta("older content\n界界e\u{301}XYZ");

    let preview = app.thinking_lines(8)[1].to_string();

    assert_eq!(preview, "  界e\u{301}XYZ");
    assert!(!preview.contains('\n'));
}

#[test]
fn new_thinking_block_resets_expanded_archive_view() {
    let mut app = App::new();
    app.show_thinking = true;
    app.on_thinking_delta("first thought");
    app.on_turn_complete();
    app.toggle_thinking();
    assert!(app.thinking_blocks[0].expanded);

    app.on_thinking_delta("next thought");

    assert!(!app.thinking_blocks[0].expanded);
    assert_eq!(app.thinking_lines(80).len(), 2);
}

#[test]
fn whitespace_only_thinking_has_single_empty_preview_row() {
    let mut app = App::new();
    app.show_thinking = true;
    app.on_thinking_delta(" \n\t\n");

    let rendered = app.thinking_lines(80);

    assert_eq!(rendered.len(), 2);
    assert_eq!(rendered[1].spans.len(), 1);
    assert!(!rendered[1].spans[0].content.contains('\n'));
}

#[test]
fn expanding_after_completion_expands_the_marker_inline() {
    let mut app = App::new();
    app.show_thinking = true;
    app.on_text_delta("before");
    app.on_thinking_delta("archived thought");
    app.on_turn_complete();

    app.toggle_thinking();

    let lines = app.output.all_lines();
    assert_eq!(lines[0], "before");
    assert!(lines[1].starts_with("✻ Thought for "));
    assert_eq!(lines[2], "  archived thought");
    assert_eq!(lines[3], "");
    assert!(app.thinking_lines(80).is_empty());

    app.toggle_thinking();

    let lines = app.output.all_lines();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "before");
    assert!(lines[1].starts_with("✻ Thought for "));
    assert_eq!(lines[2], "");
}

#[test]
fn expanding_an_older_block_keeps_newer_marker_inline() {
    let mut app = App::new();
    app.on_thinking_delta("first thought");
    app.on_turn_complete();
    app.on_thinking_delta("second thought");
    app.on_turn_complete();

    app.open_thinking_archive();
    app.select_previous_thinking_block();
    app.expand_selected_thinking_block();
    app.toggle_thinking();

    let lines = app.output.all_lines();
    let second_marker = lines
        .iter()
        .position(|line| line.starts_with("✻ Thought for"))
        .and_then(|first_marker| {
            lines
                .iter()
                .enumerate()
                .skip(first_marker + 1)
                .find(|(_, line)| line.starts_with("✻ Thought for"))
                .map(|(index, _)| index)
        })
        .expect("second marker must remain in the transcript");

    assert_eq!(lines[second_marker + 1], "  second thought");
}

#[test]
fn thinking_archive_overlay_renders_older_blocks_and_selection() {
    let mut app = App::new();
    app.show_splash = false;
    app.on_thinking_delta("first thought");
    app.on_turn_complete();
    app.on_thinking_delta("second thought");
    app.on_turn_complete();
    app.open_thinking_archive();
    app.select_previous_thinking_block();

    let backend = TestBackend::new(80, 20);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|frame| super::render::draw(frame, &mut app))
        .unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect::<String>();

    assert!(screen.contains("Thinking archive"));
    assert!(screen.contains("first thought"));
    assert!(screen.contains("second thought"));
}

#[test]
fn thinking_archive_keys_navigate_expand_and_close() {
    use crossterm::event::KeyCode;

    let mut app = App::new();
    app.on_thinking_delta("first thought");
    app.on_turn_complete();
    app.on_thinking_delta("second thought");
    app.on_turn_complete();
    app.open_thinking_archive();

    super::event_loop::thinking_archive::handle_key(&mut app, KeyCode::Up);
    assert_eq!(app.thinking_archive_selection(), Some(0));

    super::event_loop::thinking_archive::handle_key(&mut app, KeyCode::Enter);
    assert!(app.thinking_archive_selection().is_none());
    assert!(
        app.output
            .all_lines()
            .iter()
            .any(|line| *line == "  first thought")
    );
}

#[test]
fn thinking_archive_navigation_selects_older_blocks() {
    let mut app = App::new();
    app.on_thinking_delta("first thought");
    app.on_turn_complete();
    app.on_thinking_delta("second thought");
    app.on_turn_complete();

    app.open_thinking_archive();
    app.select_previous_thinking_block();

    assert_eq!(app.thinking_archive_selection(), Some(0));
    assert_eq!(
        app.thinking_archive_block()
            .map(|block| block.text.as_str()),
        Some("first thought")
    );
}
