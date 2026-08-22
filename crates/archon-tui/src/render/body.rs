//! Body rendering.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{
        Block, Borders, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Wrap,
    },
};

use crate::app::App;
use crate::splash;

use super::cursor::set_input_cursor;
use super::layout::{input_scroll_for_cursor, wrapped_cursor_position};

mod pickers;
pub use pickers::{
    draw_branch_picker, draw_file_picker, draw_hooks_menu, draw_mcp_manager, draw_memory_browser,
    draw_message_selector, draw_model_picker, draw_permission_presets, draw_permissions_browser,
    draw_search_results, draw_session_picker, draw_settings_screen, draw_skills_menu,
    draw_task_overlay, draw_theme_screen, draw_thinking_archive, draw_token_attribution,
    draw_voice_capture,
};

/// Render the output area (top section with scrollable content).
pub fn draw_output_area(frame: &mut Frame, app: &App, area: Rect) {
    if app.show_splash {
        splash::draw_splash(
            frame.buffer_mut(),
            area,
            &app.splash_model,
            &app.splash_working_dir,
            &app.splash_activity,
        );
        return;
    }

    let output_area =
        crate::agent_activity::render_rail_if_needed(frame, &app.agent_activity, area, &app.theme);
    draw_output_content(frame, app, output_area);
}

fn draw_output_content(frame: &mut Frame, app: &App, output_area: Rect) {
    let output_width = output_area.width.saturating_sub(1);
    let thinking_lines = app.thinking_lines(output_width);
    let regions = output_regions(app, output_area);
    let (transcript_area, footer_area) = transcript_regions(app, regions.transcript);
    draw_transcript(frame, app, output_width, transcript_area, footer_area);
    draw_active_thinking(frame, app, &thinking_lines, regions.thinking);
}

fn draw_transcript(frame: &mut Frame, app: &App, width: u16, area: Rect, footer_area: Rect) {
    let view = app.output.rendered_view(&app.theme, width, area.height);
    let new_rows = app
        .output
        .new_wrapped_rows(view.total_wrapped, width, &app.theme);
    let border_color = if app.output.scroll_locked {
        app.theme.warning
    } else {
        app.theme.border
    };
    let widget = Paragraph::new(view.lines)
        .block(
            Block::default()
                .borders(Borders::NONE)
                .style(Style::default().fg(border_color)),
        )
        .wrap(Wrap { trim: false })
        .scroll((view.paragraph_scroll_y, 0));
    let paragraph_area = Rect { width, ..area };
    frame.render_widget(widget, paragraph_area);
    draw_output_scrollbar(frame, app, area, view.total_wrapped, view.global_scroll_y);
    draw_locked_footer(frame, app, footer_area, new_rows);
}

fn draw_active_thinking(frame: &mut Frame, app: &App, lines: &[Line<'static>], area: Rect) {
    if area.height == 0 {
        return;
    }
    let start = app.thinking.effective_scroll(lines.len(), area.height);
    let end = start.saturating_add(area.height as usize).min(lines.len());
    frame.render_widget(Paragraph::new(lines[start..end].to_vec()), area);
}

fn draw_output_scrollbar(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    total_wrapped: usize,
    scroll_y: usize,
) {
    if total_wrapped <= area.height as usize {
        return;
    }
    let mut state =
        ScrollbarState::new(total_wrapped.saturating_sub(area.height as usize)).position(scroll_y);
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .style(Style::default().fg(app.theme.muted));
    frame.render_stateful_widget(scrollbar, area, &mut state);
}

fn draw_locked_footer(frame: &mut Frame, app: &App, area: Rect, new_rows: usize) {
    if !app.output.scroll_locked || new_rows == 0 {
        return;
    }
    let footer = Paragraph::new(format!("▼ {new_rows} new lines — PageDown/End to follow"))
        .style(Style::default().fg(app.theme.warning));
    frame.render_widget(footer, area);
}

pub(crate) fn transcript_regions(app: &App, area: Rect) -> (Rect, Rect) {
    if !app.output.scroll_locked || area.height == 0 {
        return (area, Rect::new(area.x, area.bottom(), area.width, 0));
    }
    let transcript = Rect {
        height: area.height.saturating_sub(1),
        ..area
    };
    let footer = Rect {
        y: transcript.bottom(),
        width: area.width.saturating_sub(1),
        height: 1,
        ..area
    };
    (transcript, footer)
}

#[derive(Clone, Copy)]
pub(crate) struct OutputRegions {
    pub transcript: Rect,
    pub thinking: Rect,
}

pub(crate) fn output_regions(app: &App, output_area: Rect) -> OutputRegions {
    let output_width = output_area.width.saturating_sub(1);
    let thinking_lines = app.thinking_lines(output_width);
    let thinking_height =
        reserved_thinking_height(&thinking_lines, output_area.height, app.thinking.expanded);
    let transcript_height = output_area
        .height
        .saturating_sub(thinking_height)
        .max((output_area.height > 0) as u16);
    OutputRegions {
        transcript: Rect {
            height: transcript_height,
            ..output_area
        },
        thinking: Rect {
            y: output_area.y.saturating_add(transcript_height),
            height: output_area.height.saturating_sub(transcript_height),
            ..output_area
        },
    }
}

fn reserved_thinking_height(lines: &[Line<'_>], visible_height: u16, expanded: bool) -> u16 {
    if lines.is_empty() || visible_height == 0 {
        return 0;
    }
    let cap = if expanded {
        thinking_height_cap(visible_height)
    } else {
        visible_height.saturating_sub(1)
    };
    lines.len().min(cap as usize) as u16
}

fn thinking_height_cap(visible_height: u16) -> u16 {
    visible_height
        .div_ceil(3)
        .min(visible_height.saturating_sub(1))
}

pub(crate) fn input_prefix(app: &App) -> String {
    if app.is_generating {
        match &app.active_tool {
            Some(tool) => format!("[{tool}] > "),
            None => "[...] > ".to_string(),
        }
    } else {
        "> ".to_string()
    }
}

pub(crate) fn input_display_text(app: &App) -> String {
    if let Some(ref tool) = app.permission_prompt {
        format!("Allow {tool}? [y/n]")
    } else if let Some(ref question) = app.ask_user_prompt {
        format!("{question}\n> {}", app.ask_user_draft)
    } else if let Some(ref vim) = app.vim_state {
        let mode_indicator = vim.mode_display();
        let display_line = vim.text().lines().last().unwrap_or("").to_string();
        format!("{mode_indicator} {display_line}")
    } else {
        format!("{}{}", input_prefix(app), app.input.text())
    }
}

pub(crate) fn input_text_before_cursor(app: &App) -> String {
    let cursor = app.input.cursor().min(app.input.text().len());
    let before = app.input.text().get(..cursor).unwrap_or(app.input.text());
    format!("{}{}", input_prefix(app), before)
}

pub(crate) fn input_scroll_y(app: &App, area: Rect) -> u16 {
    if app.permission_prompt.is_some() || app.ask_user_prompt.is_some() || app.vim_state.is_some() {
        return 0;
    }
    let visible_rows = area.height.saturating_sub(1);
    let (cursor_row, _) = wrapped_cursor_position(&input_text_before_cursor(app), area.width);
    input_scroll_for_cursor(cursor_row, visible_rows)
}

/// Render the input area (middle section with input line).
pub fn draw_input_area(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let input_border_style = Style::default().fg(if app.is_generating {
        t.border
    } else {
        t.border_active
    });
    let scroll_y = input_scroll_y(app, area);

    let input_widget = if app.input.ultrathink.active {
        // Build per-character rainbow spans for ultrathink keywords
        let text = app.input.text();
        let prefix_span = ratatui::text::Span::raw(input_prefix(app));
        let mut spans = vec![prefix_span];
        for (byte_idx, ch) in text.char_indices() {
            if let Some(color) = app.input.ultrathink.color_at(byte_idx) {
                spans.push(ratatui::text::Span::styled(
                    String::from(ch),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(ratatui::text::Span::raw(String::from(ch)));
            }
        }
        Paragraph::new(Line::from(spans))
            .wrap(Wrap { trim: false })
            .scroll((scroll_y, 0))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(input_border_style),
            )
    } else if let Some(ref tool) = app.permission_prompt {
        Paragraph::new(format!("Allow {tool}? [y/n]"))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
    } else if let Some(ref question) = app.ask_user_prompt {
        Paragraph::new(format!("{question}\n> {}", app.ask_user_draft))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(Style::default().fg(Color::Yellow)),
            )
            .style(Style::default().fg(Color::Yellow))
    } else if let Some(ref vim) = app.vim_state {
        let mode_indicator = vim.mode_display();
        let vim_text = vim.text();
        let display_line = vim_text.lines().last().unwrap_or("").to_string();
        Paragraph::new(format!("{mode_indicator} {display_line}"))
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(input_border_style),
            )
            .style(Style::default().fg(t.accent))
    } else {
        Paragraph::new(input_display_text(app))
            .wrap(Wrap { trim: false })
            .scroll((scroll_y, 0))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(input_border_style),
            )
            .style(Style::default().fg(t.fg))
    };

    frame.render_widget(input_widget, area);
    set_input_cursor(frame, app, area);
}

/// Render the session name badge (right-aligned on input line).
///
/// The badge is sized by *display width*, not by `str::len`: a session name
/// with any non-ASCII byte used to reserve more columns than it paints, which
/// pushed the badge off its right-aligned anchor and left the cells between
/// the badge and the margin owned by nobody.
pub fn draw_session_badge(frame: &mut Frame, app: &App, input_area: Rect) {
    let name = match &app.session_name {
        Some(n) => n,
        None => return,
    };

    // One column of margin on each side of the badge, inside the input area.
    let available = input_area.width.saturating_sub(2) as usize;
    let badge = super::width::truncate_to_width(&format!(" {name} "), available);
    let badge_width = super::width::display_width(&badge) as u16;
    if badge_width == 0 {
        return;
    }
    let badge_x = input_area.right().saturating_sub(badge_width + 1);
    let badge_area = Rect::new(badge_x, input_area.y, badge_width, 1);
    let badge_widget =
        Paragraph::new(badge).style(Style::default().fg(Color::Black).bg(Color::Cyan));
    frame.render_widget(badge_widget, badge_area);
}

/// Render the command suggestion popup (above the input line).
pub fn draw_suggestions_popup(frame: &mut Frame, app: &App, input_area: Rect) {
    if !app.input.suggestions.active || app.is_generating {
        return;
    }

    let t = &app.theme;
    let suggestions = &app.input.suggestions.suggestions;
    let visible_count = suggestions.len().min(8);
    if visible_count == 0 {
        return;
    }

    let selected = app.input.suggestions.selected_index;
    let items: Vec<ListItem<'_>> = suggestions
        .iter()
        .take(8)
        .enumerate()
        .map(|(i, cmd)| {
            let style = if i == selected {
                Style::default().fg(t.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.fg)
            };
            let desc_style = if i == selected {
                Style::default().fg(t.fg)
            } else {
                Style::default().fg(t.muted)
            };
            // `{:<16}` pads by char count, which is a column count only while
            // the name is narrow text; pad in columns so the description
            // column lines up whatever the name contains.
            let line = Line::from(vec![
                ratatui::text::Span::styled(super::width::fit_to_width(&cmd.name, 16), style),
                ratatui::text::Span::styled(
                    super::width::fit_to_width(cmd.kind.label(), 10),
                    desc_style,
                ),
                ratatui::text::Span::raw(" "),
                ratatui::text::Span::styled(cmd.description.as_str(), desc_style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let popup_height = (visible_count as u16) + 2;
    let popup_y = input_area.y.saturating_sub(popup_height);
    let popup_width = input_area.width.min(60);
    let popup_area = Rect::new(input_area.x, popup_y, popup_width, popup_height);

    // The popup floats over the output area, so its footprint has to be cells
    // it owns rather than cells it happens to overlap: `Clear` resets them
    // before the list is drawn (#174 part 2, point 2).
    frame.render_widget(ratatui::widgets::Clear, popup_area);

    let popup = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Commands ")
            .border_style(Style::default().fg(t.border_active))
            .style(Style::default().fg(t.fg)),
    );
    frame.render_widget(popup, popup_area);
}

#[cfg(test)]
mod tests {
    use ratatui::text::Line;

    use super::{reserved_thinking_height, thinking_height_cap};

    #[test]
    fn expanded_thinking_is_capped_at_about_one_third_of_viewport() {
        let lines = (0..40)
            .map(|index| Line::from(format!("thought {index}")))
            .collect::<Vec<_>>();

        assert_eq!(thinking_height_cap(30), 10);
        assert_eq!(reserved_thinking_height(&lines, 30, true), 10);
        assert_eq!(thinking_height_cap(31), 11);
    }
}
