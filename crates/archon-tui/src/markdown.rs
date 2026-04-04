//! Markdown-to-ratatui renderer.
//!
//! Provides two rendering modes:
//!
//! - [`render_markdown`] — Full document parser using [`pulldown_cmark`].
//!   Handles headings, bold, italic, inline code, fenced code blocks
//!   (syntax-highlighted via [`crate::syntax`]), blockquotes, lists, and links.
//!
//! - [`render_markdown_line`] — Legacy single-line renderer using theme colors.
//!   Retained for backward compatibility with the existing TUI rendering
//!   pipeline in `app.rs`.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::syntax::{highlight_code, render_plain_code};
use crate::theme::Theme;

// ===========================================================================
// Full-document markdown renderer (new)
// ===========================================================================

/// Parse markdown text into styled ratatui Lines.
///
/// Supports headings, bold, italic, inline code, fenced code blocks
/// (syntax highlighted), blockquotes, ordered/unordered lists, and links.
pub fn render_markdown(text: &str) -> Vec<Line<'static>> {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(text, opts);
    let mut renderer = MarkdownRenderer::new();
    for event in parser {
        renderer.push_event(event);
    }
    renderer.finish()
}

// ---------------------------------------------------------------------------
// Renderer internals
// ---------------------------------------------------------------------------

struct MarkdownRenderer {
    /// Completed output lines.
    lines: Vec<Line<'static>>,
    /// Spans accumulating for the current line.
    current_spans: Vec<Span<'static>>,
    /// Stack of active styles (bold, italic, heading, etc.).
    style_stack: Vec<Style>,
    /// Whether we are inside a fenced code block.
    in_code_block: bool,
    /// Language tag for the current code block, if any.
    code_block_lang: Option<String>,
    /// Accumulated raw text of the current code block.
    code_block_content: String,
    /// Whether we are inside a blockquote.
    in_blockquote: bool,
    /// Current ordered-list item index (None when not in an ordered list).
    ordered_list_index: Option<u64>,
    /// Whether we are inside an unordered list.
    in_unordered_list: bool,
    /// URL of the link currently being rendered.
    link_url: Option<String>,
    /// Whether we are inside a table header row.
    in_table_head: bool,
    /// Accumulated cell contents for the current table row.
    table_row_cells: Vec<Vec<Span<'static>>>,
}

impl MarkdownRenderer {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            current_spans: Vec::new(),
            style_stack: Vec::new(),
            in_code_block: false,
            code_block_lang: None,
            code_block_content: String::new(),
            in_blockquote: false,
            ordered_list_index: None,
            in_unordered_list: false,
            link_url: None,
            in_table_head: false,
            table_row_cells: Vec::new(),
        }
    }

    /// Current effective style (top of the stack, or default).
    fn current_style(&self) -> Style {
        self.style_stack.last().copied().unwrap_or_default()
    }

    /// Flush current spans into a completed line.
    fn flush_line(&mut self) {
        let spans = std::mem::take(&mut self.current_spans);
        self.lines.push(Line::from(spans));
    }

    /// Process a single pulldown-cmark event.
    fn push_event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.on_start(tag),
            Event::End(tag_end) => self.on_end(tag_end),

            Event::Text(text) => {
                if self.in_code_block {
                    self.code_block_content.push_str(&text);
                } else {
                    let style = self.current_style();
                    self.current_spans
                        .push(Span::styled(text.to_string(), style));
                }
            }

            Event::Code(code) => {
                let style = Style::default()
                    .fg(Color::Yellow)
                    .bg(Color::Rgb(40, 40, 50));
                self.current_spans
                    .push(Span::styled(code.to_string(), style));
            }

            Event::HardBreak | Event::SoftBreak => {
                if !self.in_code_block {
                    self.flush_line();
                }
            }

            // Ignore HTML, footnotes, task-list markers, etc.
            _ => {}
        }
    }

    fn on_start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                let style = heading_style(level as u8);
                self.style_stack.push(style);
            }
            Tag::Strong => {
                let mut style = self.current_style();
                style.add_modifier |= Modifier::BOLD;
                self.style_stack.push(style);
            }
            Tag::Emphasis => {
                let mut style = self.current_style();
                style.add_modifier |= Modifier::ITALIC;
                self.style_stack.push(style);
            }
            Tag::BlockQuote(_) => {
                self.in_blockquote = true;
            }
            Tag::List(start) => {
                if let Some(n) = start {
                    self.ordered_list_index = Some(n);
                } else {
                    self.in_unordered_list = true;
                }
            }
            Tag::Item => {
                if let Some(n) = self.ordered_list_index {
                    self.current_spans.push(Span::raw(format!("  {n}. ")));
                } else if self.in_unordered_list {
                    self.current_spans
                        .push(Span::raw("  \u{2022} ".to_string()));
                }
            }
            Tag::CodeBlock(kind) => {
                self.in_code_block = true;
                self.code_block_content.clear();
                self.code_block_lang = match kind {
                    CodeBlockKind::Fenced(lang) => {
                        let lang = lang.trim().to_string();
                        if lang.is_empty() {
                            None
                        } else {
                            Some(lang)
                        }
                    }
                    CodeBlockKind::Indented => None,
                };
            }
            Tag::Link { dest_url, .. } => {
                self.link_url = Some(dest_url.to_string());
                let mut style = self.current_style();
                style.add_modifier |= Modifier::UNDERLINED;
                style.fg = Some(Color::Cyan);
                self.style_stack.push(style);
            }
            Tag::Table(_alignments) => {
                // Table start — nothing to do, rows handle themselves.
            }
            Tag::TableHead => {
                self.in_table_head = true;
                self.table_row_cells.clear();
            }
            Tag::TableRow => {
                self.table_row_cells.clear();
            }
            Tag::TableCell => {
                // Start collecting cell content into current_spans.
                // We'll transfer to table_row_cells on TagEnd::TableCell.
                if self.in_table_head {
                    let mut style = self.current_style();
                    style.add_modifier |= Modifier::BOLD;
                    self.style_stack.push(style);
                }
            }
            Tag::Paragraph => {
                // Separate paragraphs with a blank line when there is prior content.
                if !self.lines.is_empty() || !self.current_spans.is_empty() {
                    self.flush_line();
                }
            }
            _ => {}
        }
    }

    fn on_end(&mut self, tag_end: TagEnd) {
        match tag_end {
            TagEnd::Heading(_) => {
                self.style_stack.pop();
                self.flush_line();
            }
            TagEnd::Strong | TagEnd::Emphasis => {
                self.style_stack.pop();
            }
            TagEnd::BlockQuote(_) => {
                self.in_blockquote = false;
            }
            TagEnd::List(_) => {
                self.ordered_list_index = None;
                self.in_unordered_list = false;
            }
            TagEnd::Item => {
                self.flush_line();
                // Advance ordered-list counter.
                if let Some(ref mut n) = self.ordered_list_index {
                    *n += 1;
                }
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                let code = std::mem::take(&mut self.code_block_content);
                let lang = self.code_block_lang.take();

                // Trim trailing newline that pulldown-cmark appends.
                let code = code.trim_end_matches('\n');

                let highlighted = if let Some(ref lang) = lang {
                    // Try tree-sitter highlighting; fall back to plain code.
                    highlight_code(code, lang)
                        .unwrap_or_else(|| render_plain_code(code, Some(lang)))
                } else {
                    // No language tag: plain monospace fallback.
                    render_plain_code(code, None)
                };

                self.lines.extend(highlighted);
            }
            TagEnd::Link => {
                self.style_stack.pop();
                if let Some(url) = self.link_url.take() {
                    self.current_spans.push(Span::styled(
                        format!(" ({url})"),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
            }
            TagEnd::Table => {
                // Add a blank line after the table.
                self.lines.push(Line::from(Vec::<Span>::new()));
            }
            TagEnd::TableHead => {
                // Emit header row with pipe separators.
                self.emit_table_row();
                // Emit separator line.
                let sep: String = self
                    .table_row_cells
                    .iter()
                    .map(|_| "------")
                    .collect::<Vec<_>>()
                    .join("-+-");
                self.lines.push(Line::from(Span::styled(
                    format!("| {sep} |"),
                    Style::default().fg(Color::DarkGray),
                )));
                self.in_table_head = false;
                self.table_row_cells.clear();
            }
            TagEnd::TableRow => {
                self.emit_table_row();
                self.table_row_cells.clear();
            }
            TagEnd::TableCell => {
                if self.in_table_head {
                    self.style_stack.pop();
                }
                let cell_spans = std::mem::take(&mut self.current_spans);
                self.table_row_cells.push(cell_spans);
            }
            TagEnd::Paragraph => {
                if self.in_blockquote {
                    // Prefix blockquote lines with a vertical bar.
                    let spans = std::mem::take(&mut self.current_spans);
                    let mut prefixed = vec![Span::styled(
                        "\u{2502} ".to_string(),
                        Style::default().fg(Color::DarkGray),
                    )];
                    prefixed.extend(spans);
                    self.lines.push(Line::from(prefixed));
                } else {
                    self.flush_line();
                }
            }
            _ => {}
        }
    }

    /// Emit a table row from accumulated cell spans with pipe separators.
    fn emit_table_row(&mut self) {
        let sep_style = Style::default().fg(Color::DarkGray);
        let mut row_spans: Vec<Span<'static>> = vec![Span::styled("| ".to_string(), sep_style)];
        for (i, cell) in self.table_row_cells.iter().enumerate() {
            if i > 0 {
                row_spans.push(Span::styled(" | ".to_string(), sep_style));
            }
            row_spans.extend(cell.iter().cloned());
        }
        row_spans.push(Span::styled(" |".to_string(), sep_style));
        self.lines.push(Line::from(row_spans));
    }

    /// Consume the renderer and return all accumulated lines.
    fn finish(mut self) -> Vec<Line<'static>> {
        if !self.current_spans.is_empty() {
            self.flush_line();
        }
        self.lines
    }
}

/// Return the style for a heading at the given level (1-6).
fn heading_style(level: u8) -> Style {
    match level {
        1 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        2 => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
        3 => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    }
}

// ===========================================================================
// Legacy single-line renderer (backward compatible)
// ===========================================================================

/// Render a single line of markdown into styled ratatui Spans using theme
/// colors.
///
/// This is the original line-by-line renderer retained for backward
/// compatibility with the streaming output display in `app.rs`. For full
/// document rendering prefer [`render_markdown`].
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

    while let Some(ch) = chars.next() {
        if ch == '`' {
            if !current.is_empty() {
                spans.push(Span::raw(std::mem::take(&mut current)));
            }
            let mut code = String::new();
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
    fn heading_style_levels() {
        let h1 = heading_style(1);
        assert_eq!(h1.fg, Some(Color::Cyan));
        assert!(h1.add_modifier.contains(Modifier::BOLD));

        let h2 = heading_style(2);
        assert_eq!(h2.fg, Some(Color::Blue));

        let h3 = heading_style(3);
        assert_eq!(h3.fg, Some(Color::Green));

        let h4 = heading_style(4);
        assert_eq!(h4.fg, Some(Color::White));
    }

    #[test]
    fn renderer_empty_input() {
        let lines = render_markdown("");
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.content.to_string())
            .collect();
        assert!(text.trim().is_empty());
    }

    // Legacy renderer tests -------------------------------------------------

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
