//! Markdown message formatter for chat messages.
//! Delegates to crate::markdown::render_markdown_line for actual parsing.
//! Layer 1 module — no imports from screens/ or app/.

use crate::theme::Theme;
use ratatui::text::{Line, Span};

/// Render a chat message (user or assistant) as formatted lines.
/// Adds role header and indentation. Delegates markdown parsing.
pub fn render_message(text: &str, role: &str, theme: &Theme) -> Vec<Line<'static>> {
    use crate::markdown::render_markdown_line;

    let role_label = match role {
        "user" => "You",
        "assistant" => "Assistant",
        "system" => "System",
        _ => role,
    };

    let header = match role {
        "user" => Span::styled(format!("\n{}: ", role_label), theme.header),
        "assistant" => Span::styled(format!("\n{}: ", role_label), theme.accent),
        _ => Span::styled(format!("\n[{}] ", role_label), theme.border),
    };

    let body = render_markdown_line(text, theme);
    let mut result = vec![Line::from(header)];
    result.push(body);
    result
}
