//! Mouse dispatch helpers for the TUI event loop.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::app::App;

pub(super) fn handle_mouse_event(app: &mut App, mouse: MouseEvent) {
    match mouse.kind {
        MouseEventKind::ScrollUp if app.activity_stream.is_foreground() => {
            app.activity_stream.scroll_up();
        }
        MouseEventKind::ScrollDown if app.activity_stream.is_foreground() => {
            app.activity_stream.scroll_down();
        }
        MouseEventKind::ScrollUp => scroll_for_pointer(app, mouse.row, true),
        MouseEventKind::ScrollDown => scroll_for_pointer(app, mouse.row, false),
        MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
            let _ = scroll_output_from_bar(app, mouse.column, mouse.row);
        }
        _ => {}
    }
}

fn scroll_for_pointer(app: &mut App, row: u16, up: bool) {
    let Some(output_area) = current_output_area(app) else {
        scroll_output(app, up);
        return;
    };
    scroll_for_region(app, output_area, row, up);
}

fn scroll_for_region(app: &mut App, output_area: Rect, row: u16, up: bool) {
    let regions = crate::render::body::output_regions(app, output_area);
    let over_thinking = row >= regions.thinking.y && row < regions.thinking.bottom();
    if over_thinking && app.thinking.active && app.thinking.expanded {
        if up {
            app.thinking.scroll_up(8);
        } else {
            app.thinking.scroll_down(8);
        }
    } else {
        scroll_output(app, up);
    }
}

fn scroll_output(app: &mut App, up: bool) {
    if up {
        app.output.scroll_up(8);
    } else {
        app.output.scroll_down(8);
    }
}

fn scroll_output_from_bar(app: &mut App, column: u16, row: u16) -> bool {
    let Some(output_area) = current_output_area(app) else {
        return false;
    };
    let output_regions = crate::render::body::output_regions(app, output_area);
    let (area, _) = crate::render::body::transcript_regions(app, output_regions.transcript);
    if !is_output_scrollbar_hit(area, column, row) {
        return false;
    }
    let width = area.width.saturating_sub(1).max(1);
    let view = app.output.rendered_view(&app.theme, width, area.height);
    if view.total_wrapped <= area.height as usize {
        return false;
    }
    app.output.scroll_to_viewport_row(
        view.total_wrapped,
        area.height,
        row.saturating_sub(area.y),
        area.height,
    );
    true
}

fn output_area_for_size(app: &App, size: Rect) -> Rect {
    let input_height = crate::render::layout::input_height_for_display(
        size,
        &crate::render::body::input_display_text(app),
    );
    let layout = crate::render::layout::compute_layout_with_input_height(size, input_height);
    output_area_before_activity_rail(app, layout.output)
}

fn current_output_area(app: &App) -> Option<Rect> {
    let (cols, rows) = crossterm::terminal::size().ok()?;
    Some(output_area_for_size(app, Rect::new(0, 0, cols, rows)))
}

fn output_area_before_activity_rail(app: &App, area: Rect) -> Rect {
    if app.agent_activity.is_empty() || area.height < 10 {
        return area;
    }
    let rail_height = (app.agent_activity.len() as u16 + 2).clamp(3, 10);
    let rail_height = rail_height.min(area.height.saturating_sub(3));
    Rect {
        height: area.height.saturating_sub(rail_height),
        ..area
    }
}

fn is_output_scrollbar_hit(area: Rect, column: u16, row: u16) -> bool {
    let inside_rows = row >= area.y && row < area.y.saturating_add(area.height);
    let near_right_edge = column >= area.right().saturating_sub(2);
    inside_rows && near_right_edge
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expanded_thinking_receives_mouse_scroll_without_moving_transcript() {
        let mut app = App::new();
        app.show_thinking = true;
        app.thinking.active = true;
        app.thinking.expanded = true;
        app.thinking.accumulated = (0..30)
            .map(|index| format!("thought-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let output_area = Rect::new(0, 2, 100, 30);
        let thinking_row = crate::render::body::output_regions(&app, output_area)
            .thinking
            .y;

        scroll_for_region(&mut app, output_area, thinking_row, true);

        assert_eq!(app.thinking.scroll_offset, 8);
        assert_eq!(app.output.scroll_offset, 0);
    }

    #[test]
    fn multiline_input_uses_same_transcript_scrollbar_geometry_as_rendering() {
        let mut app = App::new();
        app.show_thinking = true;
        app.thinking.active = true;
        app.thinking.expanded = true;
        app.thinking.accumulated = (0..30)
            .map(|index| format!("thought-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.input.set_text(&"long input ".repeat(200));
        let size = Rect::new(0, 0, 40, 30);
        let output_area = output_area_for_size(&app, size);
        let transcript = crate::render::body::output_regions(&app, output_area).transcript;

        assert!(output_area.height < 23, "dynamic input must shrink output");
        assert!(!is_output_scrollbar_hit(
            transcript,
            transcript.right().saturating_sub(1),
            transcript.bottom()
        ));
    }

    #[test]
    fn thinking_rows_are_not_transcript_scrollbar_hits() {
        let mut app = App::new();
        app.show_thinking = true;
        app.thinking.active = true;
        app.thinking.expanded = true;
        app.thinking.accumulated = (0..30)
            .map(|index| format!("thought-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let output_area = Rect::new(0, 2, 100, 30);
        let transcript = crate::render::body::output_regions(&app, output_area).transcript;

        assert!(transcript.height < output_area.height);
        assert!(is_output_scrollbar_hit(
            transcript,
            99,
            transcript.bottom().saturating_sub(1)
        ));
        assert!(!is_output_scrollbar_hit(
            transcript,
            99,
            transcript.bottom()
        ));
    }

    #[test]
    fn wheel_scroll_routes_to_region_under_pointer() {
        let mut app = App::new();
        app.show_thinking = true;
        app.thinking.active = true;
        app.thinking.expanded = true;
        app.thinking.accumulated = (0..30)
            .map(|index| format!("thought-{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let output_area = Rect::new(0, 2, 100, 30);
        let transcript = crate::render::body::output_regions(&app, output_area).transcript;

        scroll_for_region(&mut app, output_area, transcript.y, true);
        assert_eq!(app.output.scroll_offset, 8);
        assert_eq!(app.thinking.scroll_offset, 0);

        scroll_for_region(&mut app, output_area, transcript.bottom(), true);
        assert_eq!(app.output.scroll_offset, 8);
        assert_eq!(app.thinking.scroll_offset, 8);
    }

    #[test]
    fn scrollbar_hit_accepts_right_edge_inside_output_rows() {
        let area = Rect::new(0, 2, 100, 20);
        assert!(is_output_scrollbar_hit(area, 99, 2));
        assert!(is_output_scrollbar_hit(area, 98, 21));
        assert!(!is_output_scrollbar_hit(area, 97, 2));
        assert!(!is_output_scrollbar_hit(area, 99, 22));
    }
}
