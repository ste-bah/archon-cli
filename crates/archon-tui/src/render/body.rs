//! Body rendering — output area, input area, overlays (core TUI draw pipeline).

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

use crate::app::{App, McpManagerView};
use crate::markdown::render_markdown_line;
use crate::output::OutputBuffer;
use crate::splash;

use super::cursor::set_input_cursor;

/// Render the output area (top section with scrollable content).
pub fn draw_output_area(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;

    // Output area — splash or buffer lines + thinking indicator
    let output_lines: Vec<Line<'_>> = if app.show_splash {
        splash::render_splash(
            &app.splash_model,
            &app.splash_working_dir,
            &app.splash_activity,
        )
    } else {
        let mut lines: Vec<Line<'_>> = app
            .output
            .all_lines()
            .iter()
            .map(|line| render_markdown_line(line, &app.theme))
            .collect();
        lines.extend(app.thinking_lines());
        lines
    };

    let visible_height = area.height;
    let output_width = area.width.saturating_sub(1); // -1 for scrollbar
    let raw_strings: Vec<String> = output_lines
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.as_ref())
                .collect::<String>()
        })
        .collect();
    let raw_refs: Vec<&str> = raw_strings.iter().map(|s| s.as_str()).collect();
    let total_wrapped = OutputBuffer::count_wrapped_rows(&raw_refs, output_width);

    let scroll_y = if app.show_splash {
        0
    } else {
        app.output.effective_scroll(total_wrapped, visible_height)
    };

    // Border color signals when the user is scrolled away from bottom
    let border_style = if app.output.scroll_locked {
        Style::default().fg(t.warning)
    } else {
        Style::default().fg(t.border)
    };

    let output_widget = Paragraph::new(output_lines)
        .block(Block::default().borders(Borders::NONE).style(border_style))
        .wrap(Wrap { trim: false })
        .scroll((scroll_y, 0));
    frame.render_widget(output_widget, area);

    // Scrollbar — only shown when content exceeds visible area
    if total_wrapped > visible_height && !app.show_splash {
        let mut scrollbar_state =
            ScrollbarState::new(total_wrapped.saturating_sub(visible_height) as usize)
                .position(scroll_y as usize);
        let scrollbar =
            Scrollbar::new(ScrollbarOrientation::VerticalRight).style(Style::default().fg(t.muted));
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

/// Render the input area (middle section with input line).
pub fn draw_input_area(frame: &mut Frame, app: &App, area: Rect) {
    let t = &app.theme;
    let input_border_style = Style::default().fg(if app.is_generating {
        t.border
    } else {
        t.border_active
    });

    let input_widget = if app.input.ultrathink.active {
        // Build per-character rainbow spans for ultrathink keywords
        let text = app.input.text();
        let prefix_span = ratatui::text::Span::raw("> ");
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
        Paragraph::new(Line::from(spans)).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(input_border_style),
        )
    } else if let Some(ref tool) = app.permission_prompt {
        Paragraph::new(format!("Allow {tool}? [y/n]"))
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
    } else if let Some(ref vim) = app.vim_state {
        let mode_indicator = vim.mode_display();
        let vim_text = vim.text();
        let display_line = vim_text.lines().last().unwrap_or("").to_string();
        Paragraph::new(format!("{mode_indicator} {display_line}"))
            .block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(input_border_style),
            )
            .style(Style::default().fg(t.accent))
    } else {
        let prefix = if app.is_generating {
            match &app.active_tool {
                Some(tool) => format!("[{tool}] > "),
                None => "[...] > ".to_string(),
            }
        } else {
            "> ".to_string()
        };
        Paragraph::new(format!("{prefix}{}", app.input.text()))
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

/// Render the session picker overlay.
pub fn draw_session_picker(frame: &mut Frame, app: &App) {
    let picker = match &app.session_picker {
        Some(p) => p,
        None => return,
    };

    let t = &app.theme;
    let area = frame.area();
    let overlay_width = (area.width * 9 / 10).max(70).min(area.width - 2);
    let overlay_height = (picker.sessions.len() as u16 + 3)
        .min(area.height - 4)
        .max(8);
    let x = (area.width.saturating_sub(overlay_width)) / 2;
    let y = (area.height.saturating_sub(overlay_height)) / 2;
    let overlay_area = Rect::new(x, y, overlay_width, overlay_height);

    frame.render_widget(ratatui::widgets::Clear, overlay_area);

    let items: Vec<ListItem<'_>> = picker
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == picker.selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(t.fg)
            };
            let id_short = &s.id[..8.min(s.id.len())];
            let name = if s.name.is_empty() { "-" } else { &s.name };
            let line = format!(
                " {id_short} | {name:15} | {:5} turns | ${:5.2} | {}",
                s.turns, s.cost, s.last_active
            );
            ListItem::new(line).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" /resume — select session (Up/Down navigate, Enter select, Esc cancel) ")
            .border_style(Style::default().fg(t.accent)),
    );
    frame.render_widget(list, overlay_area);
}

/// Render the message-selector overlay (TASK-TUI-620 /rewind).
///
/// Delegates to `MessageSelector::render` with the full frame area so
/// the overlay sizes itself (centered, ~9/10 wide) identically to
/// `draw_session_picker` above.
pub fn draw_message_selector(frame: &mut Frame, app: &App) {
    let sel = match &app.message_selector {
        Some(s) => s,
        None => return,
    };
    let area = frame.area();
    sel.render(frame, area, &app.theme);
}

/// Render the skills-menu overlay (TASK-TUI-627 /skills).
///
/// Delegates to `SkillsMenu::render` with the full frame area so the
/// overlay sizes itself (centered, ~9/10 wide) identically to
/// `draw_message_selector` above.
pub fn draw_skills_menu(frame: &mut Frame, app: &App) {
    let menu = match &app.skills_menu {
        Some(m) => m,
        None => return,
    };
    let area = frame.area();
    menu.render(frame, area, &app.theme);
}

/// Render the file-picker overlay (TASK-#207 SLASH-FILES).
///
/// Delegates to `FilePicker::render` with the full frame area so the
/// overlay sizes itself (centered, ~9/10 wide) identically to the
/// other overlays.
pub fn draw_file_picker(frame: &mut Frame, app: &App) {
    let picker = match &app.file_picker {
        Some(p) => p,
        None => return,
    };
    let area = frame.area();
    picker.render(frame, area, &app.theme);
}

/// Render the search-results overlay (TASK-#208 SLASH-SEARCH).
pub fn draw_search_results(frame: &mut Frame, app: &App) {
    let sr = match &app.search_results {
        Some(s) => s,
        None => return,
    };
    let area = frame.area();
    sr.render(frame, area, &app.theme);
}

/// Render the MCP manager overlay.
pub fn draw_mcp_manager(frame: &mut Frame, app: &App) {
    let mcp_mgr = match &app.mcp_manager {
        Some(m) => m,
        None => return,
    };

    let t = &app.theme;
    let area = frame.area();
    let overlay_width = (area.width * 3 / 4).max(60).min(area.width - 2);
    let x = (area.width.saturating_sub(overlay_width)) / 2;

    match &mcp_mgr.view {
        McpManagerView::ServerList { selected } => {
            let overlay_height = (mcp_mgr.servers.len() as u16 + 3)
                .max(5)
                .min(area.height.saturating_sub(4));
            let y = (area.height.saturating_sub(overlay_height)) / 2;
            let overlay_area = Rect::new(x, y, overlay_width, overlay_height);
            frame.render_widget(ratatui::widgets::Clear, overlay_area);

            let items: Vec<ListItem<'_>> = mcp_mgr
                .servers
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let icon = match s.state.as_str() {
                        "ready" => "✓",
                        "crashed" => "✗",
                        "starting" | "restarting" => "⋯",
                        "disabled" => "⊘",
                        _ => "○",
                    };
                    let icon_style = match s.state.as_str() {
                        "ready" => Style::default().fg(t.accent),
                        "crashed" => Style::default().fg(Color::Red),
                        "starting" | "restarting" => Style::default().fg(Color::Yellow),
                        _ => Style::default().fg(t.muted),
                    };
                    let row_style = if i == *selected {
                        Style::default()
                            .fg(Color::Black)
                            .bg(t.accent)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(t.fg)
                    };
                    let tool_str = if s.tool_count > 0 {
                        format!(" ({} tools)", s.tool_count)
                    } else {
                        String::new()
                    };
                    let line = Line::from(vec![
                        ratatui::text::Span::styled(format!(" {} ", icon), icon_style),
                        ratatui::text::Span::styled(format!("{}{}", s.name, tool_str), row_style),
                        ratatui::text::Span::styled(
                            format!("  [{}]", s.state),
                            Style::default().fg(t.muted),
                        ),
                    ]);
                    ListItem::new(line)
                })
                .collect();

            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" MCP Servers (↑↓ navigate, Enter select, Esc close) ")
                    .border_style(Style::default().fg(t.accent)),
            );
            frame.render_widget(list, overlay_area);
        }
        McpManagerView::ServerMenu {
            server_idx,
            action_idx,
        } => {
            if let Some(server) = mcp_mgr.servers.get(*server_idx) {
                let actions = crate::event_loop::mcp_actions_for(server);
                let overlay_height = (actions.len() as u16 + 4)
                    .max(6)
                    .min(area.height.saturating_sub(4));
                let y = (area.height.saturating_sub(overlay_height)) / 2;
                let overlay_area = Rect::new(x, y, overlay_width, overlay_height);
                frame.render_widget(ratatui::widgets::Clear, overlay_area);

                let status_line = Line::from(vec![
                    ratatui::text::Span::styled("  State: ", Style::default().fg(t.muted)),
                    ratatui::text::Span::styled(server.state.clone(), Style::default().fg(t.fg)),
                ]);

                let action_items: Vec<ListItem<'_>> = actions
                    .iter()
                    .enumerate()
                    .map(|(i, act)| {
                        let label = match *act {
                            "reconnect" => "  Reconnect",
                            "disable" => "  Disable",
                            "enable" => "  Enable",
                            "tools" => "  View Tools",
                            "back" => "  Back",
                            _ => act,
                        };
                        let style = if i == *action_idx {
                            Style::default()
                                .fg(Color::Black)
                                .bg(t.accent)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(t.fg)
                        };
                        ListItem::new(Line::from(ratatui::text::Span::styled(label, style)))
                    })
                    .collect();

                let mut all_items = vec![ListItem::new(status_line), ListItem::new(Line::from(""))];
                all_items.extend(action_items);

                let list = List::new(all_items).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} ", server.name))
                        .border_style(Style::default().fg(t.accent)),
                );
                frame.render_widget(list, overlay_area);
            }
        }
        McpManagerView::ToolList {
            server_name,
            tools,
            scroll,
        } => {
            let visible = (area.height.saturating_sub(6)) as usize;
            let overlay_height = (visible as u16 + 4)
                .min(area.height.saturating_sub(4))
                .max(6);
            let y = (area.height.saturating_sub(overlay_height)) / 2;
            let overlay_area = Rect::new(x, y, overlay_width, overlay_height);
            frame.render_widget(ratatui::widgets::Clear, overlay_area);

            let items: Vec<ListItem<'_>> = if tools.is_empty() {
                vec![ListItem::new(Line::from(ratatui::text::Span::styled(
                    "  (no tools)",
                    Style::default().fg(t.muted),
                )))]
            } else {
                tools
                    .iter()
                    .skip(*scroll)
                    .take(visible)
                    .map(|name| {
                        ListItem::new(Line::from(vec![
                            ratatui::text::Span::styled("  • ", Style::default().fg(t.accent)),
                            ratatui::text::Span::styled(name.clone(), Style::default().fg(t.fg)),
                        ]))
                    })
                    .collect()
            };

            let title = format!(" {} — tools (↑↓ scroll, Esc back) ", server_name);
            let list = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(Style::default().fg(t.accent)),
            );
            frame.render_widget(list, overlay_area);
        }
    }
}
