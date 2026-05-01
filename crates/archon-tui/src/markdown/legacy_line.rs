//! Legacy single-line markdown renderer using theme colors.
//!
//! Relocated from `src/markdown.rs` (legacy-renderer section, lines 359-474
//! + tests 510-537) per REM-2c. Private to the `markdown` module; public
//!   entry via [`render_markdown_line`] re-exported from `markdown::mod`.
//!
//! Retained for backward compatibility with the streaming output display
//! in `app.rs`. For full-document rendering prefer
//! [`super::render_markdown`].

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Render a single line of markdown into styled ratatui Spans using theme
/// colors.
///
/// This is the original line-by-line renderer retained for backward
/// compatibility with the streaming output display in `app.rs`. For full
/// document rendering prefer [`super::render_markdown`].
pub fn render_markdown_line(text: &str, theme: &Theme) -> Line<'static> {
    // Headers
    if let Some(rest) = text.strip_prefix("### ") {
        return Line::from(Span::styled(
            rest.to_string(),
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(theme.accent),
        ));
    }
    if let Some(rest) = text.strip_prefix("## ") {
        return Line::from(Span::styled(
            rest.to_string(),
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(theme.header),
        ));
    }
    if let Some(rest) = text.strip_prefix("# ") {
        return Line::from(Span::styled(
            rest.to_string(),
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(theme.accent_secondary),
        ));
    }

    // Code block markers
    if text.starts_with("```") {
        let lang = text.strip_prefix("```").unwrap_or("").trim();
        if lang.is_empty() {
            return Line::from(Span::styled(
                "---".to_string(),
                Style::default().fg(theme.muted),
            ));
        }
        return Line::from(Span::styled(
            format!("[{lang}]"),
            Style::default().fg(theme.muted),
        ));
    }

    // List items
    if text.starts_with("- ") || text.starts_with("* ") {
        let rest = &text[2..];
        return Line::from(vec![
            Span::styled("  - ".to_string(), Style::default().fg(theme.muted)),
            Span::raw(rest.to_string()),
        ]);
    }

    // Inline bold and code
    let spans = parse_inline_styles(text, theme);
    Line::from(spans)
}

/// Parse inline `**bold**` and `` `code` `` markers using theme colors.
fn parse_inline_styles(text: &str, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();

    // yield_now not needed: sync loop, not cancellable regardless.
    while let Some(ch) = chars.next() {
        if ch == '`' {
            if !current.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut current)));
            }
            let mut code = String::new();
            // yield_now not needed: sync loop, not cancellable regardless.
            for c in chars.by_ref() {
                if c == '`' {
                    break;
                }
                code.push(c);
            }
            spans.push(Span::styled(code, Style::default().fg(theme.success)));
        } else if ch == '*' && chars.peek() == Some(&'*') {
            chars.next(); // consume second *
            if !current.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut current)));
            }
            let mut bold = String::new();
            // yield_now not needed: sync loop, not cancellable regardless.
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'*') {
                    chars.next();
                    break;
                }
                bold.push(c);
            }
            spans.push(Span::styled(
                bold,
                Style::default().add_modifier(Modifier::BOLD),
            ));
        } else {
            current.push(ch);
        }
    }

    if !current.is_empty() {
        spans.push(Span::raw(current));
    }

    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::intj_theme;

    #[test]
    fn legacy_header_rendering() {
        let t = intj_theme();
        let line = render_markdown_line("# Title", &t);
        assert!(!line.spans.is_empty());
    }

    #[test]
    fn legacy_code_block_marker() {
        let t = intj_theme();
        let line = render_markdown_line("```rust", &t);
        let text: String = line.spans.iter().map(|s| s.content.to_string()).collect();
        assert!(text.contains("rust"));
    }

    #[test]
    fn legacy_inline_code() {
        let t = intj_theme();
        let spans = parse_inline_styles("use `foo` here", &t);
        assert!(spans.len() >= 3);
    }

    #[test]
    fn legacy_inline_bold() {
        let t = intj_theme();
        let spans = parse_inline_styles("this is **bold** text", &t);
        assert!(spans.len() >= 3);
    }
}
