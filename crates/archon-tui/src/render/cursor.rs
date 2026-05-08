//! TUI cursor positioning helpers.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::App;

use super::body::{input_scroll_y, input_text_before_cursor};
use super::layout::wrapped_cursor_position;

pub(crate) fn set_input_cursor(frame: &mut Frame, app: &App, area: Rect) {
    // #250: ratatui requires explicit cursor positioning per frame.
    if area.height < 2 || area.width == 0 || app.permission_prompt.is_some() {
        return;
    }

    if let Some(ref vim) = app.vim_state {
        let col: u16 = vim.cursor().1.try_into().unwrap_or(u16::MAX);
        let cursor_x = area.x + vim.mode_display().chars().count() as u16 + 1 + col;
        frame.set_cursor_position((cursor_x.min(area.right().saturating_sub(1)), area.y + 1));
    } else {
        let (cursor_row, cursor_col) =
            wrapped_cursor_position(&input_text_before_cursor(app), area.width);
        let scroll_y = input_scroll_y(app, area);
        let visible_rows = area.height.saturating_sub(1).max(1);
        let visible_row = cursor_row.saturating_sub(scroll_y).min(visible_rows - 1);
        frame.set_cursor_position((area.x + cursor_col, area.y + 1 + visible_row));
    }
}
