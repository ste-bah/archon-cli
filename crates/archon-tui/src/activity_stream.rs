use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::activity_stream_layout::{inset, overlay_area};
use crate::app::App;
use crate::events::{
    ActivityStreamLineKind, ActivityStreamUpdate, AgentActivityRole, AgentActivityStatus,
    AgentActivityUpdate,
};

const MAX_LINES_PER_ACTOR: usize = 400;
const MAX_ACTORS: usize = 32;

#[derive(Debug, Clone, Default)]
pub struct ActivityStreamState {
    actors: Vec<ActivityActor>,
    selected: usize,
    foreground: bool,
    scroll: u16,
    follow_latest: bool,
}

#[derive(Debug, Clone)]
struct ActivityActor {
    id: String,
    name: String,
    role: AgentActivityRole,
    status: AgentActivityStatus,
    provider: Option<String>,
    model: Option<String>,
    lines: Vec<ActivityLine>,
    updated_seq: u64,
}

#[derive(Debug, Clone)]
struct ActivityLine {
    kind: ActivityStreamLineKind,
    text: String,
    tool: Option<String>,
    is_error: bool,
}

impl ActivityStreamState {
    pub fn is_foreground(&self) -> bool {
        self.foreground
    }

    pub fn open(&mut self) {
        self.foreground = true;
        self.follow_latest = true;
        self.selected = 0;
    }

    pub fn background(&mut self) {
        self.foreground = false;
    }

    pub fn scroll_up(&mut self) {
        self.follow_latest = false;
        self.scroll = self.scroll.saturating_sub(5);
    }

    pub fn scroll_down(&mut self) {
        self.follow_latest = false;
        self.scroll = self.scroll.saturating_add(5);
    }

    pub fn scroll_top(&mut self) {
        self.follow_latest = false;
        self.scroll = 0;
    }

    pub fn scroll_bottom(&mut self) {
        self.follow_latest = true;
    }

    pub fn select_prev(&mut self) {
        self.follow_latest = false;
        self.selected = self.selected.saturating_sub(1);
        self.scroll = 0;
    }

    pub fn select_next(&mut self) {
        self.follow_latest = false;
        if !self.actors.is_empty() {
            self.selected = (self.selected + 1).min(self.actors.len() - 1);
            self.scroll = 0;
        }
    }

    pub fn apply_update(&mut self, update: ActivityStreamUpdate) {
        let updated_id = update.id.clone();
        let selected_id = (!self.follow_latest)
            .then(|| self.actors.get(self.selected).map(|actor| actor.id.clone()))
            .flatten();
        let next_seq = self.next_seq();
        let actor = self.upsert_actor(&update, next_seq);
        actor.status = update.status;
        if let Some(provider) = update.provider.clone() {
            actor.provider = Some(provider);
        }
        if let Some(model) = update.model.clone() {
            actor.model = Some(model);
        }
        actor.updated_seq = next_seq;
        if should_keep_line(&update) {
            actor.lines.push(ActivityLine {
                kind: update.kind,
                text: update.text,
                tool: update.tool,
                is_error: update.is_error,
            });
            if actor.lines.len() > MAX_LINES_PER_ACTOR {
                let overflow = actor.lines.len() - MAX_LINES_PER_ACTOR;
                actor.lines.drain(0..overflow);
            }
        }
        self.sort_actors();
        if self.follow_latest {
            if let Some(index) = self.actors.iter().position(|actor| actor.id == updated_id) {
                self.selected = index;
            }
        } else if let Some(index) =
            selected_id.and_then(|id| self.actors.iter().position(|actor| actor.id == id))
        {
            self.selected = index;
        }
    }

    pub fn push_parent(
        &mut self,
        kind: ActivityStreamLineKind,
        text: impl Into<String>,
        tool: Option<String>,
        is_error: bool,
    ) {
        self.apply_update(ActivityStreamUpdate {
            id: "parent".into(),
            name: "Parent".into(),
            role: AgentActivityRole::Parent,
            status: if is_error {
                AgentActivityStatus::Failed
            } else {
                AgentActivityStatus::Running
            },
            provider: None,
            model: None,
            kind,
            text: text.into(),
            tool,
            is_error,
        });
    }

    fn next_seq(&self) -> u64 {
        self.actors
            .iter()
            .map(|actor| actor.updated_seq)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    fn upsert_actor(&mut self, update: &ActivityStreamUpdate, seq: u64) -> &mut ActivityActor {
        if let Some(index) = self.actors.iter().position(|actor| actor.id == update.id) {
            let actor = &mut self.actors[index];
            actor.name = update.name.clone();
            actor.role = update.role;
            return actor;
        }
        if self.actors.len() >= MAX_ACTORS {
            self.actors.remove(0);
            self.selected = self.selected.saturating_sub(1);
        }
        self.actors.push(ActivityActor {
            id: update.id.clone(),
            name: update.name.clone(),
            role: update.role,
            status: update.status,
            provider: update.provider.clone(),
            model: update.model.clone(),
            lines: Vec::new(),
            updated_seq: seq,
        });
        self.selected = self.actors.len().saturating_sub(1);
        self.actors.last_mut().expect("actor was just pushed")
    }

    fn sort_actors(&mut self) {
        self.actors.sort_by_key(|actor| {
            (
                matches!(
                    actor.status,
                    AgentActivityStatus::Complete
                        | AgentActivityStatus::Failed
                        | AgentActivityStatus::Cancelled
                ),
                std::cmp::Reverse(actor.updated_seq),
            )
        });
        self.selected = self.selected.min(self.actors.len().saturating_sub(1));
    }
}

impl App {
    pub fn on_activity_stream_update(&mut self, update: ActivityStreamUpdate) {
        self.activity_stream.apply_update(update);
    }

    pub fn record_activity_update(&mut self, update: &AgentActivityUpdate) {
        self.activity_stream.apply_update(ActivityStreamUpdate {
            id: update.id.clone(),
            name: update.name.clone(),
            role: update.role,
            status: update.status,
            provider: update.provider.clone(),
            model: update.model.clone(),
            kind: ActivityStreamLineKind::Status,
            text: update
                .detail
                .clone()
                .unwrap_or_else(|| format!("{:?}", update.status).to_lowercase()),
            tool: update.current_tool.clone(),
            is_error: matches!(
                update.status,
                AgentActivityStatus::Failed | AgentActivityStatus::Cancelled
            ),
        });
    }

    pub fn open_activity_stream(&mut self) {
        self.activity_stream.open();
    }

    pub fn background_activity_stream(&mut self) {
        self.activity_stream.background();
    }

    pub fn push_parent_activity_status(&mut self, text: impl Into<String>) {
        self.activity_stream
            .push_parent(ActivityStreamLineKind::Status, text, None, false);
    }

    pub fn push_parent_activity_text(&mut self, text: &str) {
        self.activity_stream
            .push_parent(ActivityStreamLineKind::Text, text, None, false);
    }

    pub fn push_parent_activity_thinking(&mut self, text: &str) {
        self.activity_stream
            .push_parent(ActivityStreamLineKind::Thinking, text, None, false);
    }

    pub fn push_parent_activity_tool_call(&mut self, name: &str) {
        self.activity_stream.push_parent(
            ActivityStreamLineKind::ToolCall,
            format!("calling {name}"),
            Some(name.into()),
            false,
        );
    }

    pub fn push_parent_activity_tool_result(&mut self, name: &str, output: &str, is_error: bool) {
        self.activity_stream.push_parent(
            ActivityStreamLineKind::ToolResult,
            summarize(output),
            Some(name.into()),
            is_error,
        );
    }

    pub fn push_parent_activity_error(&mut self, message: &str) {
        self.activity_stream.push_parent(
            ActivityStreamLineKind::Error,
            message.to_string(),
            None,
            true,
        );
    }
}

pub fn draw_activity_overlay(frame: &mut Frame, app: &App) {
    if !app.activity_stream.foreground {
        return;
    }
    let area = overlay_area(frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default()
        .title(" Agent Activity  PgUp/PgDn scroll  Ctrl+End follow  Ctrl+B background ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(app.theme.border_active));
    frame.render_widget(block, area);
    let inner = inset(area, 1, 1);
    if inner.width < 70 {
        render_stream(frame, app, inner);
        return;
    }
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(32), Constraint::Min(20)])
        .split(inner);
    render_actor_list(frame, app, chunks[0]);
    render_stream(frame, app, chunks[1]);
}

fn render_actor_list(frame: &mut Frame, app: &App, area: Rect) {
    let state = &app.activity_stream;
    let mut lines = Vec::new();
    if state.actors.is_empty() {
        lines.push(Line::from(Span::styled(
            "No agent activity yet",
            Style::default().fg(app.theme.muted),
        )));
    }
    for (index, actor) in state.actors.iter().enumerate() {
        let selected = index == state.selected;
        let marker = if selected { "> " } else { "  " };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(app.theme.accent)),
            Span::styled(
                role_label(actor.role),
                status_style(actor.status, &app.theme),
            ),
            Span::raw(" "),
            Span::styled(actor.name.clone(), name_style(selected, &app.theme)),
        ]));
        lines.push(Line::from(Span::styled(
            actor_subtitle(actor),
            Style::default().fg(app.theme.muted),
        )));
    }
    let widget = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::RIGHT))
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}

fn render_stream(frame: &mut Frame, app: &App, area: Rect) {
    let state = &app.activity_stream;
    let Some(actor) = state.actors.get(state.selected) else {
        let empty = Paragraph::new("No foreground activity. Ctrl+B returns to chat.")
            .style(Style::default().fg(app.theme.muted));
        frame.render_widget(empty, area);
        return;
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(actor.name.clone(), Style::default().fg(app.theme.header)),
        Span::raw(" "),
        Span::styled(
            status_text(actor.status),
            status_style(actor.status, &app.theme),
        ),
        Span::raw("  "),
        Span::styled(actor_subtitle(actor), Style::default().fg(app.theme.muted)),
    ])];
    lines.push(Line::raw(""));
    for item in &actor.lines {
        lines.extend(line_rows(item, &app.theme));
    }
    let max_scroll = lines
        .len()
        .saturating_sub(area.height as usize)
        .min(u16::MAX as usize) as u16;
    let scroll = if state.follow_latest {
        max_scroll
    } else {
        state.scroll.min(max_scroll)
    };
    let widget = Paragraph::new(Text::from(lines))
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}

fn line_rows<'a>(item: &ActivityLine, theme: &crate::theme::Theme) -> Vec<Line<'a>> {
    let label = match item.kind {
        ActivityStreamLineKind::Status => "status",
        ActivityStreamLineKind::Thinking => "thinking",
        ActivityStreamLineKind::Text => "assistant",
        ActivityStreamLineKind::ToolCall => "tool call",
        ActivityStreamLineKind::ToolResult => "tool result",
        ActivityStreamLineKind::FinalOutput => "final",
        ActivityStreamLineKind::Error => "error",
    };
    let color = if item.is_error {
        Color::Red
    } else {
        match item.kind {
            ActivityStreamLineKind::Thinking => theme.muted,
            ActivityStreamLineKind::ToolCall | ActivityStreamLineKind::ToolResult => Color::Yellow,
            ActivityStreamLineKind::Error => Color::Red,
            _ => theme.fg,
        }
    };
    let mut rows = Vec::new();
    rows.push(Line::from(vec![
        Span::styled(label, Style::default().fg(theme.accent)),
        Span::raw(
            item.tool
                .as_ref()
                .map(|tool| format!(" {tool}"))
                .unwrap_or_default(),
        ),
    ]));
    for line in item.text.lines().take(16) {
        rows.push(Line::from(Span::styled(
            format!("  {line}"),
            Style::default().fg(color),
        )));
    }
    rows
}

fn should_keep_line(update: &ActivityStreamUpdate) -> bool {
    update.kind != ActivityStreamLineKind::Status
        || !update.text.trim().is_empty()
        || update.status != AgentActivityStatus::Running
}

fn role_label(role: AgentActivityRole) -> &'static str {
    match role {
        AgentActivityRole::Parent => "[PARENT]",
        AgentActivityRole::Subagent => "[AGENT]",
        AgentActivityRole::Background => "[BG]",
    }
}

fn status_text(status: AgentActivityStatus) -> &'static str {
    match status {
        AgentActivityStatus::Queued => "queued",
        AgentActivityStatus::Running => "running",
        AgentActivityStatus::Waiting => "waiting",
        AgentActivityStatus::WaitingForTool => "tool",
        AgentActivityStatus::Backgrounded => "background",
        AgentActivityStatus::Complete => "done",
        AgentActivityStatus::Failed => "failed",
        AgentActivityStatus::Cancelled => "cancelled",
    }
}

fn status_style(status: AgentActivityStatus, theme: &crate::theme::Theme) -> Style {
    let color = match status {
        AgentActivityStatus::Complete => Color::Green,
        AgentActivityStatus::Failed => Color::Red,
        AgentActivityStatus::Cancelled => Color::DarkGray,
        AgentActivityStatus::Waiting | AgentActivityStatus::WaitingForTool => Color::Yellow,
        AgentActivityStatus::Backgrounded => Color::Cyan,
        AgentActivityStatus::Queued => Color::DarkGray,
        AgentActivityStatus::Running => theme.accent,
    };
    Style::default().fg(color).add_modifier(Modifier::BOLD)
}

fn name_style(selected: bool, theme: &crate::theme::Theme) -> Style {
    let style = Style::default().fg(theme.fg);
    if selected {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn actor_subtitle(actor: &ActivityActor) -> String {
    match (&actor.provider, &actor.model) {
        (Some(provider), Some(model)) => format!("{provider} / {model}"),
        (None, Some(model)) => model.clone(),
        (Some(provider), None) => provider.clone(),
        (None, None) => status_text(actor.status).into(),
    }
}

fn summarize(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.chars().count() <= 500 {
        return trimmed.to_string();
    }
    let mut summary: String = trimmed.chars().take(500).collect();
    summary.push_str("...");
    summary
}

#[cfg(test)]
#[path = "activity_stream_tests.rs"]
mod tests;
