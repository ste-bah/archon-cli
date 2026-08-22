use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem},
};

use crate::app::{App, McpManagerView, format_duration};

pub fn draw_thinking_archive(frame: &mut Frame, app: &App) {
    let Some(selected) = app.thinking_archive_selection() else {
        return;
    };

    let area = frame.area();
    let width = area.width.saturating_sub(2).min(72);
    let height = (app.thinking_blocks.len() as u16 + 2)
        .max(5)
        .min(area.height.saturating_sub(2));
    let overlay = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(ratatui::widgets::Clear, overlay);

    let items = app
        .thinking_blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let preview = block
                .text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("(empty)");
            let style = if index == selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(app.theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.theme.fg)
            };
            ListItem::new(format!(
                " {}  {}  {}",
                index + 1,
                format_duration(block.duration_ms),
                preview
            ))
            .style(style)
        })
        .collect::<Vec<_>>();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Thinking archive (Up/Down, Enter expand, Esc close) ")
            .border_style(Style::default().fg(app.theme.accent)),
    );
    frame.render_widget(list, overlay);
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
pub fn draw_message_selector(frame: &mut Frame, app: &App) {
    let sel = match &app.message_selector {
        Some(s) => s,
        None => return,
    };
    let area = frame.area();
    sel.render(frame, area, &app.theme);
}

/// Render the skills-menu overlay (TASK-TUI-627 /skills).
pub fn draw_skills_menu(frame: &mut Frame, app: &App) {
    let menu = match &app.skills_menu {
        Some(m) => m,
        None => return,
    };
    let area = frame.area();
    menu.render(frame, area, &app.theme);
}

/// Render the model-picker overlay (#192, `/model` with no arguments).
pub fn draw_model_picker(frame: &mut Frame, app: &App) {
    let picker = match &app.model_picker {
        Some(p) => p,
        None => return,
    };
    picker.render(frame, frame.area(), &app.theme);
}

/// Render the theme picker overlay (#192, `/theme` with no arguments).
pub fn draw_theme_screen(frame: &mut Frame, app: &App) {
    let screen = match &app.theme_screen {
        Some(s) => s,
        None => return,
    };
    screen.render(frame, frame.area(), &app.theme);
}

/// Render the settings overlay (#192, `/config` with no arguments).
pub fn draw_settings_screen(frame: &mut Frame, app: &App) {
    let screen = match &app.settings_screen {
        Some(s) => s,
        None => return,
    };
    screen.render(frame, frame.area(), &app.theme);
}

/// Render the hooks overlay (#192, `/hooks` with no subcommand).
pub fn draw_hooks_menu(frame: &mut Frame, app: &App) {
    let menu = match &app.hooks_menu {
        Some(m) => m,
        None => return,
    };
    menu.render(frame, frame.area(), &app.theme);
}

/// Render the permission-rules overlay (#192, `/permissions`).
pub fn draw_permissions_browser(frame: &mut Frame, app: &App) {
    let browser = match &app.permissions_browser {
        Some(b) => b,
        None => return,
    };
    browser.render(frame, frame.area(), &app.theme);
}

/// Render the permission-preset selector (#200 Phase 3, `/permissions presets`).
pub fn draw_permission_presets(frame: &mut Frame, app: &App) {
    let picker = match &app.permission_presets {
        Some(p) => p,
        None => return,
    };
    picker.render(frame, frame.area(), &app.theme);
}

/// Render the memory-files overlay (#192, `/memory files`).
pub fn draw_memory_browser(frame: &mut Frame, app: &App) {
    let browser = match &app.memory_browser {
        Some(b) => b,
        None => return,
    };
    browser.render(frame, frame.area(), &app.theme);
}

/// Render the branch picker (#192, `/fork-at` with no arguments).
pub fn draw_branch_picker(frame: &mut Frame, app: &App) {
    let picker = match &app.branch_picker {
        Some(p) => p,
        None => return,
    };
    picker.render(frame, frame.area(), &app.theme);
}

/// Render the `@`-mention picker (#200 Phase 4).
pub fn draw_session_mention(frame: &mut Frame, app: &App) {
    let picker = match &app.session_mention {
        Some(p) => p,
        None => return,
    };
    picker.render(frame, frame.area(), &app.theme);
}

/// Render the token attribution overlay (#192 scope B, `/context`).
pub fn draw_token_attribution(frame: &mut Frame, app: &App) {
    let overlay = match &app.token_attribution {
        Some(o) => o,
        None => return,
    };
    overlay.render(frame, frame.area(), &app.theme);
}

/// Render the voice capture overlay (#192, `/voice` and the record hotkey).
pub fn draw_voice_capture(frame: &mut Frame, app: &App) {
    let overlay = match &app.voice_capture {
        Some(o) => o,
        None => return,
    };
    overlay.render(frame, frame.area(), &app.theme);
}

/// Render the tasks overlay (#189 Phase 9, Ctrl+K).
pub fn draw_task_overlay(frame: &mut Frame, app: &App) {
    let overlay = match &app.task_overlay {
        Some(o) => o,
        None => return,
    };
    let area = frame.area();
    overlay.render(frame, area, &app.theme);
}

/// Render the file-picker overlay (TASK-#207 SLASH-FILES).
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
