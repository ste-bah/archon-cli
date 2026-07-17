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
    draw_file_picker, draw_mcp_manager, draw_message_selector, draw_search_results,
    draw_session_picker, draw_skills_menu, draw_thinking_archive,
};

/// Render the output area (top section with scrollable content).
pub fn draw_output_area(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;

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

    let output_width = output_area.width.saturating_sub(1); // -1 for scrollbar
    let thinking_lines = app.thinking_lines(output_width);
    let regions = output_regions(app, output_area);
    let transcript_area = regions.transcript;
    let transcript_height = transcript_area.height;
    let rendered_view = app
        .output
        .rendered_view(&app.theme, output_width, transcript_height);
    let total_wrapped = rendered_view.total_wrapped;
    let scroll_y = rendered_view.global_scroll_y;

    let border_style = if app.output.scroll_locked {
        Style::default().fg(t.warning)
    } else {
        Style::default().fg(t.border)
    };

    let output_widget = Paragraph::new(rendered_view.lines)
        .block(Block::default().borders(Borders::NONE).style(border_style))
        .wrap(Wrap { trim: false })
        .scroll((rendered_view.paragraph_scroll_y, 0));
    frame.render_widget(output_widget, transcript_area);

    if regions.thinking.height > 0 {
        let start = app
            .thinking
            .effective_scroll(thinking_lines.len(), regions.thinking.height);
        let end = start
            .saturating_add(regions.thinking.height as usize)
            .min(thinking_lines.len());
        let thinking_widget = Paragraph::new(thinking_lines[start..end].to_vec());
        frame.render_widget(thinking_widget, regions.thinking);
    }

    if total_wrapped > transcript_height {
        let mut scrollbar_state =
            ScrollbarState::new(total_wrapped.saturating_sub(transcript_height) as usize)
                .position(scroll_y as usize);
        let scrollbar =
            Scrollbar::new(ScrollbarOrientation::VerticalRight).style(Style::default().fg(t.muted));
        frame.render_stateful_widget(scrollbar, transcript_area, &mut scrollbar_state);
    }
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
pub fn draw_session_badge(frame: &mut Frame, app: &App, input_area: Rect) {
    let name = match &app.session_name {
        Some(n) => n,
        None => return,
    };

    let badge = format!(" {name} ");
    let badge_width = badge.len() as u16;
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
            let line = Line::from(vec![
                ratatui::text::Span::styled(format!("{:<16}", cmd.name), style),
                ratatui::text::Span::styled(cmd.description.as_str(), desc_style),
            ]);
            ListItem::new(line)
        })
        .collect();

    let popup_height = (visible_count as u16) + 2;
    let popup_y = input_area.y.saturating_sub(popup_height);
    let popup_width = input_area.width.min(60);
    let popup_area = Rect::new(input_area.x, popup_y, popup_width, popup_height);

    let popup = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Commands ")
            .border_style(Style::default().fg(t.border_active))
            .style(Style::default().fg(t.fg)),
    );
    frame.render_widget(popup, popup_area);
}
