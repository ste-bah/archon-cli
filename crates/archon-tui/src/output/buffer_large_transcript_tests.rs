use super::buffer::OutputBuffer;

#[test]
fn new_line_count_survives_more_than_u16_wrapped_rows() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for _ in 0..70_000 {
        buf.append_line("existing");
    }
    buf.scroll_up(1);
    buf.append_line("new");

    let locked = buf.rendered_view(&theme, 20, 1);

    assert_eq!(locked.total_wrapped, 70_001);
    assert_eq!(locked.global_scroll_y, 69_998);
    assert_eq!(buf.new_wrapped_rows(locked.total_wrapped, 20, &theme), 1);
}

#[test]
fn locked_new_row_count_reuses_large_transcript_baseline() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..70_000 {
        buf.append_line(&format!("ROW-{index:05} | 界 | 🙂 | e\u{301}"));
    }
    let initial = buf.rendered_view(&theme, 40, 5);
    buf.scroll_up(1);

    let first_locked = buf.rendered_view(&theme, 40, 5);
    assert_eq!(first_locked.global_scroll_y, initial.global_scroll_y - 1);
    let first_work = buf.lock_baseline_line_work.get();
    assert_eq!(first_work, 70_000);

    let second_locked = buf.rendered_view(&theme, 40, 5);
    assert_eq!(second_locked.global_scroll_y, first_locked.global_scroll_y);
    assert_eq!(buf.lock_baseline_line_work.get(), first_work);
    assert_eq!(buf.new_wrapped_rows(initial.total_wrapped, 40, &theme), 0);
    assert_eq!(buf.lock_baseline_line_work.get(), first_work);
}

#[test]
fn locked_viewport_stays_fixed_when_new_rows_arrive() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..20 {
        buf.append_line(&format!("line {index}"));
    }
    buf.scroll_up(5);
    let before = buf.rendered_view(&theme, 20, 5);

    buf.append_line("new");
    let after = buf.rendered_view(&theme, 20, 5);

    assert_eq!(before.global_scroll_y, 10);
    assert_eq!(after.global_scroll_y, before.global_scroll_y);
}

#[test]
fn single_logical_line_beyond_u16_rows_renders_distinct_wide_tail() {
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::{Paragraph, Widget, Wrap};

    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    let width = 8;
    let sentinel = "T界🙂";
    let prefix = "HEAD-";
    let padding = "x".repeat(width as usize * 65_536 - prefix.len());
    buf.append_line(&format!("{prefix}{padding}{sentinel}"));

    let view = buf.rendered_view(&theme, width, 1);
    assert!(view.total_wrapped > u16::MAX as usize);
    assert_eq!(view.global_scroll_y, view.total_wrapped - 1);
    assert_eq!(view.paragraph_scroll_y, u16::MAX - 1);

    let area = Rect::new(0, 0, width, 1);
    let mut rendered = Buffer::empty(area);
    Paragraph::new(view.lines)
        .wrap(Wrap { trim: false })
        .scroll((view.paragraph_scroll_y, 0))
        .render(area, &mut rendered);

    assert_eq!(rendered[(0, 0)].symbol(), "T");
    assert_eq!(rendered[(1, 0)].symbol(), "界");
    assert_eq!(rendered[(3, 0)].symbol(), "🙂");
    assert!(rendered.content().iter().all(|cell| cell.symbol() != "H"));
}

#[test]
fn auto_scroll_beyond_u16_rows_renders_distinct_wide_tail() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..65_536 {
        buf.append_line(&format!("row-{index}"));
    }
    let sentinel = "TAIL-界界";
    buf.append_line(sentinel);

    let view = buf.rendered_view(&theme, 20, 1);

    assert!(view.total_wrapped > u16::MAX as usize);
    assert_eq!(view.global_scroll_y, view.total_wrapped - 1);
    assert_eq!(
        view.lines.last().map(ToString::to_string).as_deref(),
        Some(sentinel)
    );
}

#[test]
fn duplicate_synthetic_text_does_not_change_arrival_count() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line("marker-a");
    buf.append_line("marker-b");
    buf.insert_lines_after(0, &["same".into()]);
    buf.scroll_up(1);
    buf.append_line("new");
    buf.insert_lines_after(2, &["same".into()]);

    buf.remove_lines_after(0, 1);
    let first_collapse = buf.rendered_view(&theme, 20, 1);
    assert_eq!(
        buf.new_wrapped_rows(first_collapse.total_wrapped, 20, &theme),
        1
    );

    buf.remove_lines_after(1, 1);
    let second_collapse = buf.rendered_view(&theme, 20, 1);
    assert_eq!(
        buf.new_wrapped_rows(second_collapse.total_wrapped, 20, &theme),
        1
    );
}

#[test]
fn scroll_down_after_top_moves_and_unlocks_at_bottom() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..100 {
        buf.append_line(&format!("line {index}"));
    }
    buf.rendered_view(&theme, 20, 10);
    buf.scroll_to_top();

    buf.scroll_down(10);
    assert_eq!(buf.rendered_view(&theme, 20, 10).global_scroll_y, 10);
    assert!(buf.scroll_locked);

    for _ in 0..8 {
        buf.scroll_down(10);
    }
    assert_eq!(buf.rendered_view(&theme, 20, 10).global_scroll_y, 90);
    assert!(!buf.scroll_locked);
}

#[test]
fn count_wrapped_rows_uses_terminal_display_width() {
    let lines = ["界界"];
    assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 2), 2);
}

#[test]
fn count_wrapped_rows_word_wrap_differs_from_char_wrap() {
    let lines = ["hi hello world"];
    assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 7), 3);
}

#[test]
fn count_wrapped_rows_long_word_char_splits() {
    let lines = ["abcdefghijklmnop"];
    assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 5), 4);
}

#[test]
fn count_wrapped_rows_fits_on_one_row() {
    let lines = ["hello world"];
    assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 20), 1);
}

#[test]
fn effective_scroll_scrolled_up() {
    let mut buf = OutputBuffer::new();
    buf.scroll_locked = true;
    buf.scroll_offset = 5;
    assert_eq!(
        buf.effective_scroll(30, 10, 80, &crate::theme::intj_theme()),
        15
    );
}

#[test]
fn effective_scroll_clamped_to_zero() {
    let mut buf = OutputBuffer::new();
    buf.scroll_locked = true;
    buf.scroll_offset = 100;
    assert_eq!(
        buf.effective_scroll(30, 10, 80, &crate::theme::intj_theme()),
        0
    );
}
