use ratatui::style::{Color, Modifier, Style};

use crate::events::{ActivityStreamLineKind, AgentActivityRole, AgentActivityStatus};
use crate::theme::Theme;

const MAX_RENDERED_TEXT_LINES: usize = 160;

pub(crate) struct VisibleTextLines<'a> {
    pub omitted: usize,
    pub lines: Vec<&'a str>,
}

pub(crate) fn visible_text_lines(text: &str) -> VisibleTextLines<'_> {
    let text_lines: Vec<&str> = text.lines().collect();
    let omitted = text_lines.len().saturating_sub(MAX_RENDERED_TEXT_LINES);
    let lines = if text_lines.is_empty() {
        Vec::new()
    } else {
        text_lines.iter().skip(omitted).copied().collect()
    };
    VisibleTextLines { omitted, lines }
}

pub(crate) fn is_streaming_line(kind: ActivityStreamLineKind) -> bool {
    matches!(
        kind,
        ActivityStreamLineKind::Thinking | ActivityStreamLineKind::Text
    )
}

pub(crate) fn keeps_empty_line(kind: ActivityStreamLineKind) -> bool {
    matches!(
        kind,
        ActivityStreamLineKind::ToolCall | ActivityStreamLineKind::FinalOutput
    )
}

pub(crate) fn sanitize_activity_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            strip_escape_sequence(&mut chars);
            continue;
        }
        match ch {
            '\r' => out.push('\n'),
            '\n' => out.push('\n'),
            '\t' => out.push_str("    "),
            ch if ch.is_control() => {}
            ch => out.push(ch),
        }
    }
    out
}

fn strip_escape_sequence<I>(chars: &mut std::iter::Peekable<I>)
where
    I: Iterator<Item = char>,
{
    match chars.peek().copied() {
        Some('[') => {
            chars.next();
            for ch in chars.by_ref() {
                if ('@'..='~').contains(&ch) {
                    break;
                }
            }
        }
        Some(']') => {
            chars.next();
            while let Some(ch) = chars.next() {
                if ch == '\x07' {
                    break;
                }
                if ch == '\x1b' && matches!(chars.peek(), Some('\\')) {
                    chars.next();
                    break;
                }
            }
        }
        Some(_) => {
            chars.next();
        }
        None => {}
    }
}

pub(crate) fn role_label(role: AgentActivityRole) -> &'static str {
    match role {
        AgentActivityRole::Parent => "[PARENT]",
        AgentActivityRole::Subagent => "[AGENT]",
        AgentActivityRole::Background => "[BG]",
    }
}

pub(crate) fn status_text(status: AgentActivityStatus) -> &'static str {
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

pub(crate) fn status_style(status: AgentActivityStatus, theme: &Theme) -> Style {
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

pub(crate) fn name_style(selected: bool, theme: &Theme) -> Style {
    let style = Style::default().fg(theme.fg);
    if selected {
        style.add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

pub(crate) fn actor_subtitle(
    provider: Option<&str>,
    model: Option<&str>,
    status: AgentActivityStatus,
) -> String {
    match (provider, model) {
        (Some(provider), Some(model)) => format!("{provider} / {model}"),
        (None, Some(model)) => model.to_string(),
        (Some(provider), None) => provider.to_string(),
        (None, None) => status_text(status).into(),
    }
}

pub(crate) fn summarize(output: &str) -> String {
    let trimmed = output.trim();
    if trimmed.chars().count() <= 500 {
        return trimmed.to_string();
    }
    let mut summary: String = trimmed.chars().take(500).collect();
    summary.push_str("...");
    summary
}
