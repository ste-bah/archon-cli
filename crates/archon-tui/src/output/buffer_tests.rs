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
fn rendered_cache_extends_without_reprocessing_existing_history() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append("### Styled heading\nROW-00000 | 界 | 🙂 | e\u{301}\n");
    let first = buf.rendered_view(&theme, 40, 5);
    assert_eq!(first.total_wrapped, 2);

    for index in 1..=70_000 {
        buf.append(&format!("ROW-{index:05} | 界 | 🙂 | e\u{301}\n"));
        buf.rendered_view(&theme, 40, 5);
    }

    assert_eq!(buf.line_count(), 70_002);
    assert_eq!(buf.rendered_line_work.get(), 70_002);
    assert_eq!(buf.wrapped_line_work.get(), 70_002);
}

#[test]
fn rendered_cache_reprocesses_only_changed_partial_line() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..100 {
        buf.append_line(&format!("line {index}"));
    }
    buf.append("界");
    let first = buf.rendered_view(&theme, 2, 5);
    assert_eq!(buf.rendered_line_work.get(), 101);
    assert_eq!(buf.wrapped_line_work.get(), 101);

    buf.append("🙂e\u{301}");
    let updated = buf.rendered_view(&theme, 2, 5);

    assert_eq!(updated.total_wrapped, first.total_wrapped + 2);
    assert_eq!(
        buf.rendered_raw_lines(&theme).last().unwrap(),
        "界🙂e\u{301}"
    );
    assert_eq!(buf.rendered_line_work.get(), 102);
    assert_eq!(buf.wrapped_line_work.get(), 102);
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
fn locked_resize_preserves_logical_anchor_when_lower_lines_reflow() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..100 {
        let suffix = if index > 50 {
            " lower-content-that-wraps-differently-after-resize".repeat(3)
        } else {
            String::new()
        };
        buf.append_line(&format!("line {index:03}{suffix}"));
    }
    let wide = buf.rendered_view(&theme, 80, 6);
    buf.scroll_to_viewport_row(wide.total_wrapped, 6, 5, 11);
    let before = buf.rendered_view(&theme, 80, 6);
    let anchor = before.lines[0].to_string();

    let narrow = buf.rendered_view(&theme, 24, 6);

    assert_eq!(narrow.lines[0].to_string(), anchor);
}

#[test]
fn locked_resize_preserves_offset_within_reflowed_anchor_line() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..30 {
        buf.append_line(&format!("line {index:03} {}", "anchor-content ".repeat(8)));
    }
    let wide = buf.rendered_view(&theme, 40, 5);
    buf.scroll_to_viewport_row(wide.total_wrapped, 5, 5, 11);
    let before = buf.rendered_view(&theme, 40, 5);
    let anchor_line = buf.last_visible_line_start.get();
    let anchor_offset =
        before.global_scroll_y - buf.wrap_cache.borrow().as_ref().unwrap().offsets[anchor_line];

    let narrow = buf.rendered_view(&theme, 24, 5);

    assert_eq!(buf.last_visible_line_start.get(), anchor_line);
    assert_eq!(
        narrow.global_scroll_y - buf.wrap_cache.borrow().as_ref().unwrap().offsets[anchor_line],
        anchor_offset
    );
}

#[test]
fn repeated_locked_resize_returns_to_original_anchor() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..100 {
        let suffix = if index > 50 {
            " lower-content-that-wraps-differently-after-resize".repeat(3)
        } else {
            String::new()
        };
        buf.append_line(&format!("line {index:03}{suffix}"));
    }
    let wide = buf.rendered_view(&theme, 80, 6);
    buf.scroll_to_viewport_row(wide.total_wrapped, 6, 5, 11);
    let before = buf.rendered_view(&theme, 80, 6);
    let anchor = before.lines[0].to_string();

    assert_eq!(
        buf.rendered_view(&theme, 24, 6).lines[0].to_string(),
        anchor
    );
    assert_eq!(
        buf.rendered_view(&theme, 80, 6).lines[0].to_string(),
        anchor
    );
    assert_eq!(
        buf.rendered_view(&theme, 16, 6).lines[0].to_string(),
        anchor
    );
    assert_eq!(
        buf.rendered_view(&theme, 80, 6).lines[0].to_string(),
        anchor
    );
}

#[test]
fn scroll_to_top_clears_existing_logical_anchor() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    for index in 0..100 {
        buf.append_line(&format!("line {index}"));
    }
    let initial = buf.rendered_view(&theme, 20, 5);
    buf.scroll_to_viewport_row(initial.total_wrapped, 5, 5, 11);
    assert!(buf.rendered_view(&theme, 20, 5).global_scroll_y > 0);

    buf.scroll_to_top();

    assert_eq!(buf.rendered_view(&theme, 20, 5).global_scroll_y, 0);
}

#[test]
fn widening_clamps_anchor_offset_to_same_logical_line() {
    let theme = crate::theme::intj_theme();
    let mut buf = OutputBuffer::new();
    buf.append_line("before");
    buf.append_line(&"anchor ".repeat(12));
    buf.append_line("after");
    let narrow = buf.rendered_view(&theme, 12, 1);
    buf.scroll_to_viewport_row(narrow.total_wrapped, 1, 5, 11);
    let before = buf.rendered_view(&theme, 12, 1);
    assert_eq!(buf.last_visible_line_start.get(), 1);
    assert!(before.paragraph_scroll_y > 0);

    let wide = buf.rendered_view(&theme, 120, 1);

    assert_eq!(buf.last_visible_line_start.get(), 1);
    assert_eq!(wide.lines[0].to_string(), "anchor ".repeat(12));
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
