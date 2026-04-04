use archon_tui::virtual_scroll::{MessageBound, VirtualScroll};

#[test]
fn new_starts_at_top() {
    let vs = VirtualScroll::new(20);
    assert_eq!(vs.scroll_offset(), 0);
}

#[test]
fn scroll_down_moves_offset() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    vs.scroll_down(5);
    assert_eq!(vs.scroll_offset(), 5);
}

#[test]
fn scroll_up_clamps_at_zero() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    vs.scroll_up(100);
    assert_eq!(vs.scroll_offset(), 0);
}

#[test]
fn scroll_down_clamps_at_max() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    vs.scroll_down(200);
    // max offset = total_lines - viewport_height = 100 - 20 = 80
    assert_eq!(vs.scroll_offset(), 80);
}

#[test]
fn page_up_page_down() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    vs.page_down();
    assert_eq!(vs.scroll_offset(), 20);
    vs.page_up();
    assert_eq!(vs.scroll_offset(), 0);
}

#[test]
fn scroll_to_top() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    vs.scroll_down(50);
    vs.scroll_to_top();
    assert_eq!(vs.scroll_offset(), 0);
}

#[test]
fn scroll_to_bottom() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    vs.scroll_to_bottom();
    assert_eq!(vs.scroll_offset(), 80);
}

#[test]
fn visible_range_correct() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    vs.scroll_down(10);
    let (start, end) = vs.visible_range();
    assert_eq!(start, 10);
    assert_eq!(end, 29);
}

#[test]
fn render_range_includes_buffer() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(200, vec![]);
    vs.scroll_down(50);
    let (vis_start, vis_end) = vs.visible_range();
    let (ren_start, ren_end) = vs.render_range();
    // Render range should extend beyond visible range by buffer_lines
    assert!(ren_start <= vis_start);
    assert!(ren_end >= vis_end);
    // But clamped to content bounds
    assert!(ren_end < 200);
}

#[test]
fn auto_scroll_on_new_content() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(50, vec![]);
    // Not user-scrolled, so on_new_content should scroll to bottom
    vs.on_new_content();
    assert!(vs.at_bottom());
}

#[test]
fn user_scroll_disables_auto_scroll() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    vs.scroll_to_bottom();
    // User scrolls up
    vs.on_user_scroll();
    vs.scroll_up(10);
    // New content arrives — should NOT auto-scroll
    vs.update_content(120, vec![]);
    vs.on_new_content();
    // Should still be where the user scrolled, not at bottom
    assert!(!vs.at_bottom());
}

#[test]
fn end_key_resets_user_scroll() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    vs.on_user_scroll();
    vs.scroll_up(10);
    assert!(!vs.at_bottom());
    // Pressing End (scroll_to_bottom) should re-enable auto-scroll
    vs.scroll_to_bottom();
    assert!(vs.at_bottom());
    // Now new content should auto-scroll
    vs.update_content(120, vec![]);
    vs.on_new_content();
    assert!(vs.at_bottom());
}

#[test]
fn scroll_percentage() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    // At top: 0%
    assert!((vs.scroll_percentage() - 0.0).abs() < f32::EPSILON);
    // At halfway: offset 40 out of max 80
    vs.scroll_down(40);
    assert!((vs.scroll_percentage() - 0.5).abs() < 0.01);
    // At bottom: 100%
    vs.scroll_to_bottom();
    assert!((vs.scroll_percentage() - 1.0).abs() < f32::EPSILON);
}

#[test]
fn at_bottom_correct() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    assert!(!vs.at_bottom());
    vs.scroll_to_bottom();
    assert!(vs.at_bottom());
    vs.scroll_up(1);
    assert!(!vs.at_bottom());
}

#[test]
fn visible_messages() {
    let bounds = vec![
        MessageBound {
            start_line: 0,
            height: 10,
            message_index: 0,
        },
        MessageBound {
            start_line: 10,
            height: 15,
            message_index: 1,
        },
        MessageBound {
            start_line: 25,
            height: 20,
            message_index: 2,
        },
    ];
    let mut vs = VirtualScroll::new(10);
    vs.update_content(45, bounds);
    // Viewport at offset 12 shows lines 12..21 — overlaps message 1 (10..24)
    vs.scroll_down(12);
    let visible = vs.visible_messages();
    assert!(visible.contains(&1));
}

#[test]
fn add_message_updates_total() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(50, vec![]);
    vs.add_message(10);
    // total should now be 60
    // scroll_to_bottom should go to 60 - 20 = 40
    vs.scroll_to_bottom();
    assert_eq!(vs.scroll_offset(), 40);
}

#[test]
fn empty_content_no_panic() {
    let mut vs = VirtualScroll::new(20);
    // No content at all
    vs.scroll_up(10);
    vs.scroll_down(10);
    vs.page_up();
    vs.page_down();
    vs.scroll_to_top();
    vs.scroll_to_bottom();
    vs.on_new_content();
    vs.on_user_scroll();
    vs.reset_user_scroll();
    let _ = vs.visible_range();
    let _ = vs.render_range();
    let _ = vs.visible_messages();
    let _ = vs.scroll_percentage();
    let _ = vs.at_bottom();
    assert_eq!(vs.scroll_offset(), 0);
}

#[test]
fn viewport_larger_than_content() {
    // viewport_height > total_lines: scroll should stay at 0
    let mut vs = VirtualScroll::new(100);
    vs.update_content(10, vec![]);
    vs.scroll_down(50);
    assert_eq!(vs.scroll_offset(), 0);
    assert!(vs.at_bottom());
}

#[test]
fn set_viewport_height_works() {
    let mut vs = VirtualScroll::new(20);
    vs.update_content(100, vec![]);
    vs.scroll_down(50);
    vs.set_viewport_height(30);
    // max offset is now 100 - 30 = 70, current 50 is still valid
    assert_eq!(vs.scroll_offset(), 50);
    // But if we were past the new max, it should clamp
    vs.set_viewport_height(60);
    // max offset = 100 - 60 = 40, clamp 50 -> 40
    assert_eq!(vs.scroll_offset(), 40);
}
