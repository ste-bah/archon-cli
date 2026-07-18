use super::buffer::OutputBuffer;

#[test]
fn append_text_with_newlines() {
    let mut buf = OutputBuffer::new();
    buf.append("hello\nworld\n");
    assert_eq!(buf.all_lines(), vec!["hello", "world"]);
}

#[test]
fn append_streaming_chars() {
    let mut buf = OutputBuffer::new();
    buf.append("H");
    buf.append("e");
    buf.append("l");
    buf.append("lo");
    assert_eq!(buf.all_lines(), vec!["Hello"]);
    assert_eq!(buf.line_count(), 1);
}

#[test]
fn append_line() {
    let mut buf = OutputBuffer::new();
    buf.append_line("first");
    buf.append_line("second");
    assert_eq!(buf.all_lines(), vec!["first", "second"]);
}

#[test]
fn rendered_view_wraps_rendered_markdown_text_not_source_markup() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line("### abc");

    let view = buf.rendered_view(&theme, 4, 1);

    assert_eq!(view.total_wrapped, 1);
    assert_eq!(view.global_scroll_y, 0);
    assert_eq!(view.lines[0].to_string(), "abc");
}

#[test]
fn count_wrapped_rows_preserves_public_u16_return_type() {
    let lines = ["hello"];
    let _: u16 = OutputBuffer::count_wrapped_rows(&lines, 20);
}

#[test]
fn synthetic_expansion_above_locked_viewport_preserves_anchor() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..20 {
        buf.append_line(&format!("line {index}"));
    }
    buf.scroll_up(5);
    let before = buf.rendered_view(&theme, 20, 5);

    buf.insert_lines_after(0, &["historical detail".into()]);
    let expanded = buf.rendered_view(&theme, 20, 5);

    assert_eq!(before.global_scroll_y, 10);
    assert_eq!(expanded.global_scroll_y, 11);
    assert_eq!(expanded.lines[0].to_string(), before.lines[0].to_string());

    buf.remove_lines_after(0, 1);
    let collapsed = buf.rendered_view(&theme, 20, 5);
    assert_eq!(collapsed.global_scroll_y, before.global_scroll_y);
    assert_eq!(collapsed.lines[0].to_string(), before.lines[0].to_string());
}

#[test]
fn collapsing_preexisting_expansion_above_viewport_preserves_anchor() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..20 {
        buf.append_line(&format!("line {index}"));
    }
    buf.insert_lines_after(0, &["historical detail".into()]);
    buf.scroll_up(5);
    let before = buf.rendered_view(&theme, 20, 5);

    buf.remove_lines_after(0, 1);
    let collapsed = buf.rendered_view(&theme, 20, 5);

    assert_eq!(collapsed.global_scroll_y + 1, before.global_scroll_y);
    assert_eq!(collapsed.lines[0].to_string(), before.lines[0].to_string());
}

#[test]
fn synthetic_expansion_at_locked_viewport_boundary_preserves_anchor() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..20 {
        buf.append_line(&format!("line {index}"));
    }
    buf.scroll_up(5);
    let before = buf.rendered_view(&theme, 20, 5);
    assert_eq!(buf.last_visible_line_start.get(), 10);

    buf.insert_lines_after(9, &["historical detail".into()]);
    let expanded = buf.rendered_view(&theme, 20, 5);

    assert_eq!(expanded.global_scroll_y, before.global_scroll_y + 1);
    assert_eq!(expanded.lines[0].to_string(), before.lines[0].to_string());
}

#[test]
fn scrollbar_viewport_preserves_anchor_through_synthetic_expansion() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..100 {
        buf.append_line(&format!("line {index}"));
    }
    let initial = buf.rendered_view(&theme, 20, 10);
    buf.scroll_to_viewport_row(initial.total_wrapped, 10, 5, 11);
    let before = buf.rendered_view(&theme, 20, 10);

    buf.insert_lines_after(0, &["historical detail".into()]);
    let expanded = buf.rendered_view(&theme, 20, 10);

    assert_eq!(expanded.global_scroll_y, before.global_scroll_y + 1);
    assert_eq!(expanded.lines[0].to_string(), before.lines[0].to_string());
}

#[test]
fn rendered_lines_cache_updates_only_after_content_changes() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line("**first**");

    let first = buf.rendered_raw_lines(&theme);
    let second = buf.rendered_raw_lines(&theme);
    assert_eq!(first, second);

    buf.append_line("second");
    assert_eq!(buf.rendered_raw_lines(&theme), vec!["**first**", "second"]);
}

#[test]
fn rendered_view_clones_only_visible_lines() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for idx in 0..100 {
        buf.append_line(&format!("line {idx}"));
    }
    buf.scroll_locked = true;
    buf.scroll_offset = 50;

    let view = buf.rendered_view(&theme, 80, 5);

    assert!(view.lines.len() <= 7);
    assert_eq!(view.total_wrapped, 100);
    assert!(view.global_scroll_y > 0);
}

// -- scroll tests -------------------------------------------------------

#[test]
fn scroll_up_locks_and_increases_offset() {
    let mut buf = OutputBuffer::new();
    // scroll_offset = lines scrolled UP from bottom. scroll_up adds.
    buf.scroll_up(5);
    assert!(buf.scroll_locked);
    assert_eq!(buf.scroll_offset, 5);
    buf.scroll_up(3);
    assert_eq!(buf.scroll_offset, 8);
}

#[test]
fn scroll_down_decreases_offset() {
    let mut buf = OutputBuffer::new();
    buf.scroll_locked = true;
    buf.scroll_offset = 10;
    buf.scroll_down(3);
    assert_eq!(buf.scroll_offset, 7);
    assert!(buf.scroll_locked); // still locked, not at bottom
}

#[test]
fn scroll_down_to_zero_unlocks() {
    let mut buf = OutputBuffer::new();
    buf.scroll_locked = true;
    buf.scroll_offset = 3;
    buf.scroll_down(5); // saturating_sub: 3 - 5 = 0
    assert_eq!(buf.scroll_offset, 0);
    assert!(!buf.scroll_locked); // reached bottom, unlocked
}

#[test]
fn scroll_to_bottom_resets() {
    let mut buf = OutputBuffer::new();
    buf.scroll_locked = true;
    buf.scroll_offset = 10;
    buf.scroll_to_bottom();
    assert_eq!(buf.scroll_offset, 0);
    assert!(!buf.scroll_locked);
}

#[test]
fn effective_scroll_at_bottom() {
    let buf = OutputBuffer::new();
    // Not locked => auto-scroll to bottom => max_scroll = 30 - 10 = 20
    assert_eq!(
        buf.effective_scroll(30, 10, 80, &crate::theme::intj_theme()),
        20
    );
}

#[test]
fn new_line_count_uses_wrapped_row_delta_since_scroll_lock() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line("existing");
    let initial = buf.rendered_view(&theme, 5, 1);
    assert_eq!(buf.new_wrapped_rows(initial.total_wrapped, 5, &theme), 0);

    buf.scroll_up(1);
    buf.append_line("abcdefghij");
    let locked = buf.rendered_view(&theme, 5, 1);
    assert_eq!(buf.new_wrapped_rows(locked.total_wrapped, 5, &theme), 2);

    buf.scroll_to_bottom();
    let following = buf.rendered_view(&theme, 5, 1);
    assert_eq!(buf.new_wrapped_rows(following.total_wrapped, 5, &theme), 0);
}

#[test]
fn new_line_count_reflows_lock_snapshot_after_resize() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line("existing-content");
    let initial = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(initial.total_wrapped, 10, &theme), 0);

    buf.scroll_up(1);
    buf.append_line("abcdefghij");
    let narrow = buf.rendered_view(&theme, 5, 1);
    assert_eq!(buf.new_wrapped_rows(narrow.total_wrapped, 5, &theme), 2);

    let wide = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(wide.total_wrapped, 10, &theme), 1);
}

#[test]
fn scroll_to_top_captures_new_line_baseline() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line("existing");
    let initial = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(initial.total_wrapped, 10, &theme), 0);

    buf.scroll_to_top();
    buf.append_line("new");
    let locked = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(locked.total_wrapped, 10, &theme), 1);
}

#[test]
fn transcript_expansion_does_not_count_as_new_arrival() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line("marker");
    let initial = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(initial.total_wrapped, 10, &theme), 0);

    buf.scroll_up(1);
    buf.insert_lines_after(0, &["historical detail".into()]);
    let expanded = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(expanded.total_wrapped, 10, &theme), 0);

    buf.append_line("new");
    let arrived = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(arrived.total_wrapped, 10, &theme), 1);

    buf.remove_lines_after(0, 1);
    let collapsed = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(collapsed.total_wrapped, 10, &theme), 1);
}

#[test]
fn expansion_after_arrival_keeps_arrival_count_through_collapse() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line("marker");
    let initial = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(initial.total_wrapped, 10, &theme), 0);

    buf.scroll_up(1);
    buf.append_line("new");
    buf.insert_lines_after(1, &["historical detail".into()]);
    let expanded = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(expanded.total_wrapped, 10, &theme), 1);

    buf.remove_lines_after(1, 1);
    let collapsed = buf.rendered_view(&theme, 10, 1);
    assert_eq!(buf.new_wrapped_rows(collapsed.total_wrapped, 10, &theme), 1);
}

#[test]
fn collapse_of_expansion_present_at_lock_keeps_arrival_count() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line("marker");
    buf.insert_lines_after(0, &["historical detail".into()]);
    buf.scroll_up(1);

    buf.append_line("new");
    buf.remove_lines_after(0, 1);
    let collapsed = buf.rendered_view(&theme, 20, 1);

    assert_eq!(buf.new_wrapped_rows(collapsed.total_wrapped, 20, &theme), 1);
}

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
fn one_logical_line_can_scroll_beyond_u16_wrapped_rows() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line(&"x".repeat(70_000));

    let view = buf.rendered_view(&theme, 1, 1);

    assert_eq!(view.total_wrapped, 70_000);
    assert_eq!(view.global_scroll_y, 69_999);
    assert_eq!(view.paragraph_scroll_y, u16::MAX);
    assert_eq!(view.lines[0].to_string().chars().count(), 65_536);
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
    // "hi hello world" = 14 chars, width 7
    // Simple ceil(14/7) = 2 — WRONG for word-wrap
    // Word-wrap: "hi " (3) fits row1, "hello " (6) overflows → row2,
    //            "world" (5) overflows row2(6+5=11>7) → row3 = 3 rows
    let lines = ["hi hello world"];
    assert_eq!(OutputBuffer::count_wrapped_rows(&lines, 7), 3);
}

#[test]
fn count_wrapped_rows_long_word_char_splits() {
    // "abcdefghijklmnop" = 17 chars, width 5 → ceil(17/5) = 4 rows
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
    // max_scroll = 30 - 10 = 20. effective = 20 - 5 = 15 (scrolled 5 lines up from bottom)
    assert_eq!(
        buf.effective_scroll(30, 10, 80, &crate::theme::intj_theme()),
        15
    );
}

#[test]
fn effective_scroll_clamped_to_zero() {
    let mut buf = OutputBuffer::new();
    buf.scroll_locked = true;
    buf.scroll_offset = 100; // way past content
    // max_scroll = 30 - 10 = 20. effective = 20 - 100 = 0 (clamped via saturating_sub)
    assert_eq!(
        buf.effective_scroll(30, 10, 80, &crate::theme::intj_theme()),
        0
    );
}
