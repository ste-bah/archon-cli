//! Compact parent/subagent activity rail for the TUI.
//!
//! This deliberately mirrors the event-shape idea used by richer agent UIs
//! without copying their implementation: Archon keeps a tiny source-of-truth
//! row per active actor and renders it from local TUI state.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::events::{AgentActivityRole, AgentActivityStatus, AgentActivityUpdate};
use crate::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentActivityRow {
    pub id: String,
    pub name: String,
    pub role: AgentActivityRole,
    pub status: AgentActivityStatus,
    pub current_tool: Option<String>,
    pub detail: Option<String>,
    pub run_id: Option<String>,
    pub parent_id: Option<String>,
    pub artifact_id: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cost_usd: Option<f64>,
}

impl AgentActivityRow {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        role: AgentActivityRole,
        status: AgentActivityStatus,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            role,
            status,
            current_tool: None,
            detail: None,
            run_id: None,
            parent_id: None,
            artifact_id: None,
            provider: None,
            model: None,
            cost_usd: None,
        }
    }

    pub fn from_update(update: AgentActivityUpdate) -> Self {
        Self {
            id: update.id,
            name: update.name,
            role: update.role,
            status: update.status,
            current_tool: update.current_tool,
            detail: update.detail,
            run_id: update.run_id,
            parent_id: update.parent_id,
            artifact_id: update.artifact_id,
            provider: update.provider,
            model: update.model,
            cost_usd: update.cost_usd,
        }
    }
}

pub fn apply_update(rows: &mut Vec<AgentActivityRow>, update: AgentActivityUpdate) {
    if is_terminal_non_parent(update.role, update.status) {
        remove_row(rows, &update.id);
        return;
    }

    if let Some(row) = rows.iter_mut().find(|row| row.id == update.id) {
        row.name = update.name;
        row.role = update.role;
        row.status = update.status;
        row.current_tool = update.current_tool;
        row.detail = update.detail;
        row.run_id = update.run_id;
        row.parent_id = update.parent_id;
        row.artifact_id = update.artifact_id;
        row.provider = update.provider;
        row.model = update.model;
        row.cost_usd = update.cost_usd;
    } else {
        rows.push(AgentActivityRow::from_update(update));
    }
    trim_rows(rows);
}

pub fn turn_started(rows: &mut Vec<AgentActivityRow>) {
    upsert_parent(
        rows,
        AgentActivityStatus::Running,
        None,
        "model turn started",
    );
}

pub fn turn_completed(rows: &mut Vec<AgentActivityRow>) {
    upsert_parent(rows, AgentActivityStatus::Complete, None, "turn complete");
}

pub fn turn_failed(rows: &mut Vec<AgentActivityRow>) {
    upsert_parent(rows, AgentActivityStatus::Failed, None, "turn failed");
}

pub fn tool_started(rows: &mut Vec<AgentActivityRow>, name: &str, _id: &str) {
    upsert_parent(
        rows,
        AgentActivityStatus::WaitingForTool,
        Some(name.to_string()),
        "processing turn",
    );
}

pub fn tool_completed(rows: &mut Vec<AgentActivityRow>, name: &str, id: &str, _success: bool) {
    upsert_parent(
        rows,
        AgentActivityStatus::Running,
        None,
        "awaiting model response",
    );
    if name.eq_ignore_ascii_case("agent") {
        remove_row(rows, id);
    }
}

pub fn render_rail_if_needed(
    frame: &mut Frame,
    rows: &[AgentActivityRow],
    area: Rect,
    theme: &Theme,
) -> Rect {
    if rows.is_empty() || area.height < 10 {
        return area;
    }
    let rail_height = (rows.len() as u16 + 2).clamp(3, 10);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(rail_height.min(area.height.saturating_sub(3))),
        ])
        .split(area);
    render(frame, rows, chunks[1], theme);
    chunks[0]
}

fn upsert_parent(
    rows: &mut Vec<AgentActivityRow>,
    status: AgentActivityStatus,
    current_tool: Option<String>,
    detail: impl Into<String>,
) {
    upsert_row(
        rows,
        AgentActivityRow {
            id: "parent".into(),
            name: "Parent".into(),
            role: AgentActivityRole::Parent,
            status,
            current_tool,
            detail: Some(detail.into()),
            run_id: None,
            parent_id: None,
            artifact_id: None,
            provider: None,
            model: None,
            cost_usd: None,
        },
    );
}

pub fn render(frame: &mut Frame, rows: &[AgentActivityRow], area: Rect, theme: &Theme) {
    if rows.is_empty() || area.height == 0 || area.width < 24 {
        return;
    }

    let visible_rows = area.height.saturating_sub(2) as usize;
    let lines: Vec<Line<'_>> = rows
        .iter()
        .rev()
        .take(visible_rows)
        .rev()
        .map(|row| render_row(row, theme, area.width.saturating_sub(2) as usize))
        .collect();

    let panel = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Agent Activity ")
            .border_style(Style::default().fg(theme.border_active)),
    );
    frame.render_widget(panel, area);
}

fn upsert_row(rows: &mut Vec<AgentActivityRow>, row: AgentActivityRow) {
    if let Some(existing) = rows.iter_mut().find(|existing| existing.id == row.id) {
        *existing = row;
    } else {
        rows.push(row);
    }
    trim_rows(rows);
}

fn remove_row(rows: &mut Vec<AgentActivityRow>, id: &str) {
    rows.retain(|row| row.id != id);
}

fn is_terminal_non_parent(role: AgentActivityRole, status: AgentActivityStatus) -> bool {
    !matches!(role, AgentActivityRole::Parent)
        && matches!(
            status,
            AgentActivityStatus::Complete
                | AgentActivityStatus::Failed
                | AgentActivityStatus::Cancelled
        )
}

fn trim_rows(rows: &mut Vec<AgentActivityRow>) {
    const MAX_ROWS: usize = 12;
    if rows.len() > MAX_ROWS {
        rows.drain(0..rows.len() - MAX_ROWS);
    }
}

fn render_row<'a>(row: &AgentActivityRow, theme: &Theme, width: usize) -> Line<'a> {
    let badge = match row.role {
        AgentActivityRole::Parent => "[PARENT]",
        AgentActivityRole::Subagent => "[AGENT]",
        AgentActivityRole::Background => "[BG]",
    };
    let (status, color) = match row.status {
        AgentActivityStatus::Queued => ("queued", Color::DarkGray),
        AgentActivityStatus::Running => ("running", theme.accent),
        AgentActivityStatus::Waiting => ("waiting", Color::Yellow),
        AgentActivityStatus::WaitingForTool => ("tool", Color::Yellow),
        AgentActivityStatus::Backgrounded => ("bg", Color::Cyan),
        AgentActivityStatus::Complete => ("done", Color::Green),
        AgentActivityStatus::Failed => ("failed", Color::Red),
        AgentActivityStatus::Cancelled => ("cancel", Color::DarkGray),
    };
    let mut detail = row.detail.clone().unwrap_or_default();
    if let Some(tool) = &row.current_tool {
        detail = format!("{detail} tool={tool}");
    }
    if let Some(run_id) = &row.run_id {
        detail = format!("{detail} run={run_id}");
    }
    if let Some(artifact_id) = &row.artifact_id {
        detail = format!("{detail} artifact={artifact_id}");
    }
    if let Some(provider) = &row.provider {
        detail = format!("{detail} provider={provider}");
    }
    if let Some(model) = &row.model {
        detail = format!("{detail} model={model}");
    }
    if let Some(cost) = row.cost_usd {
        detail = format!("{detail} cost=${cost:.4}");
    }
    let text = format!(
        "{badge:<8} {status:<7} {:<18} {}",
        row.name,
        truncate(&detail, width.saturating_sub(38))
    );

    Line::from(vec![Span::styled(
        text,
        Style::default()
            .fg(color)
            .add_modifier(if matches!(row.role, AgentActivityRole::Parent) {
                Modifier::BOLD
            } else {
                Modifier::empty()
            }),
    )])
}

fn truncate(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    out.push_str("...");
    out
}
