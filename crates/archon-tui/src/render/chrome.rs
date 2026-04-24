//! Chrome rendering — status bar, permission indicator, /btw overlay.
//!
//! These are the non-body UI elements that appear in fixed regions.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

use crate::app::App;
use crate::ultrathink;

/// Render the status bar (bottom row, full width).
pub fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let status_bg = t.border;
    let status_text = app.status.format();

    let status_line = if app.input.ultrathink.active {
        let mut spans: Vec<ratatui::text::Span<'_>> = Vec::new();
        for (ch, color) in ultrathink::ultrathink_status_spans(app.input.ultrathink.shimmer_offset)
        {
            spans.push(ratatui::text::Span::styled(
                String::from(ch),
                Style::default()
                    .fg(color)
                    .bg(status_bg)
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(ratatui::text::Span::styled(
            " | ",
            Style::default()
                .fg(t.fg)
                .bg(status_bg)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(ratatui::text::Span::styled(
            status_text,
            Style::default()
                .fg(t.fg)
                .bg(status_bg)
                .add_modifier(Modifier::BOLD),
        ));
        Line::from(spans)
    } else {
        Line::from(ratatui::text::Span::styled(
            status_text,
            Style::default()
                .fg(t.fg)
                .bg(status_bg)
                .add_modifier(Modifier::BOLD),
        ))
    };

    let widget = Paragraph::new(status_line);
    frame.render_widget(widget, area);
}

/// Render the permission mode indicator (single row, just above status bar).
pub fn draw_permission_indicator(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let perm_mode = &app.status.permission_mode;
    let perm_display = match perm_mode.as_str() {
        "bypassPermissions" | "yolo" => "bypass permissions on",
        "dontAsk" => "don't ask mode",
        "acceptEdits" => "accept edits mode",
        "auto" => "auto permissions",
        "plan" => "plan mode (read-only)",
        _ => "default permissions",
    };
    let perm_line = Line::from(vec![
        ratatui::text::Span::styled(" >> ", Style::default().fg(Color::Yellow)),
        ratatui::text::Span::styled(
            perm_display,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        ratatui::text::Span::styled(" (shift+tab to cycle)", Style::default().fg(t.muted)),
    ]);
    frame.render_widget(Paragraph::new(perm_line), area);
}

/// Render the /btw overlay (centered modal).
pub fn draw_btw_overlay(frame: &mut Frame, app: &App) {
    let btw_text = match &app.btw_overlay {
        Some(t) => t,
        None => return,
    };

    let area = frame.area();
    let t = &app.theme;
    let overlay_width = (area.width * 3 / 4).max(40).min(area.width - 4);
    let lines: Vec<&str> = btw_text.lines().collect();
    let overlay_height = (lines.len() as u16 + 4).min(area.height - 4).max(5);
    let x = (area.width.saturating_sub(overlay_width)) / 2;
    let y = (area.height.saturating_sub(overlay_height)) / 2;
    let overlay_area = Rect::new(x, y, overlay_width, overlay_height);

    // Clear background
    frame.render_widget(ratatui::widgets::Clear, overlay_area);

    let text = format!("{btw_text}\n\n[Esc/Enter to dismiss]");
    let overlay = Paragraph::new(text)
        .wrap(ratatui::widgets::Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" /btw ")
                .border_style(Style::default().fg(t.accent)),
        )
        .style(Style::default().fg(t.fg));
    frame.render_widget(overlay, overlay_area);
}
