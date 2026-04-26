//! TUI cursor positioning helpers.

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::app::App;

pub(crate) fn set_input_cursor(frame: &mut Frame, app: &App, area: Rect) {
    // #250: ratatui requires explicit cursor positioning per frame.
    let cursor_x = if let Some(ref vim) = app.vim_state {
        let col: u16 = vim.cursor().1.try_into().unwrap_or(u16::MAX);
        area.x + vim.mode_display().chars().count() as u16 + 1 + col
    } else {
        let prefix = if app.is_generating {
            app.active_tool
                .as_ref()
                .map_or("[...] > ".to_string(), |t| format!("[{t}] > "))
        } else {
            "> ".to_string()
        };
        area.x + prefix.chars().count() as u16 + app.input.cursor().try_into().unwrap_or(u16::MAX)
    };
    frame.set_cursor_position((cursor_x, area.y + 1));
}
