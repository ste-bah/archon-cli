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
        MouseEventKind::ScrollUp => app.output.scroll_up(8),
        MouseEventKind::ScrollDown => app.output.scroll_down(8),
        MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left) => {
            let _ = scroll_output_from_bar(app, mouse.column, mouse.row);
        }
        _ => {}
    }
}

fn scroll_output_from_bar(app: &mut App, column: u16, row: u16) -> bool {
    let Some(area) = current_output_area(app) else {
        return false;
    };
    if !is_output_scrollbar_hit(area, column, row) {
        return false;
    }
    let width = area.width.saturating_sub(1).max(1);
    let view = app.output.rendered_view(&app.theme, width, area.height);
    if view.total_wrapped <= area.height {
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

fn current_output_area(app: &App) -> Option<Rect> {
    let (cols, rows) = crossterm::terminal::size().ok()?;
    let layout = crate::render::layout::compute_layout(Rect::new(0, 0, cols, rows));
    Some(output_area_before_activity_rail(app, layout.output))
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
    fn scrollbar_hit_accepts_right_edge_inside_output_rows() {
        let area = Rect::new(0, 2, 100, 20);
        assert!(is_output_scrollbar_hit(area, 99, 2));
        assert!(is_output_scrollbar_hit(area, 98, 21));
        assert!(!is_output_scrollbar_hit(area, 97, 2));
        assert!(!is_output_scrollbar_hit(area, 99, 22));
    }
}
