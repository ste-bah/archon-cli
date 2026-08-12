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
use crate::status::ContextWarning;
use crate::ultrathink;

use super::width::{display_width, fit_to_width};

/// Render the status bar (bottom row, full width).
///
/// The bar is rendered *width-exact*: the status text is truncated on grapheme
/// boundaries to the columns still free and then padded to fill them. The bar
/// is a single row at the bottom of the screen whose content changes on every
/// token update, so it is the surface where a computed width that disagrees
/// with the written width does the most damage — the reported #174 fragments
/// (`1`, `0`, `%`) are the shape of `ctx 392k/1000k (39%)` left half-repainted.
/// Padding also means the row is entirely cells this widget owns, so nothing
/// from a previous frame can show through the gap after the text.
pub fn draw_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let status_bg = t.border;
    let status_fg = match app.status.warning_state {
        ContextWarning::Ok => t.fg,
        ContextWarning::Warning => Color::Yellow,
        ContextWarning::Critical => Color::Red,
    };
    let status_style = Style::default()
        .fg(status_fg)
        .bg(status_bg)
        .add_modifier(Modifier::BOLD);

    let mut spans: Vec<ratatui::text::Span<'static>> = Vec::new();
    if app.input.ultrathink.active {
        spans.extend(ultrathink_prefix_spans(app, status_bg, t.fg));
    }
    let used: usize = spans.iter().map(|span| display_width(&span.content)).sum();
    let remaining = (area.width as usize).saturating_sub(used);
    spans.push(ratatui::text::Span::styled(
        fit_to_width(&app.status.format(), remaining),
        status_style,
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The shimmering `ultrathink` banner and its separator, drawn ahead of the
/// status text when ultrathink is armed.
fn ultrathink_prefix_spans(
    app: &App,
    status_bg: Color,
    separator_fg: Color,
) -> Vec<ratatui::text::Span<'static>> {
    let mut spans: Vec<ratatui::text::Span<'static>> =
        ultrathink::ultrathink_status_spans(app.input.ultrathink.shimmer_offset)
            .into_iter()
            .map(|(ch, color)| {
                ratatui::text::Span::styled(
                    String::from(ch),
                    Style::default()
                        .fg(color)
                        .bg(status_bg)
                        .add_modifier(Modifier::BOLD),
                )
            })
            .collect();
    spans.push(ratatui::text::Span::styled(
        " | ",
        Style::default()
            .fg(separator_fg)
            .bg(status_bg)
            .add_modifier(Modifier::BOLD),
    ));
    spans
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
