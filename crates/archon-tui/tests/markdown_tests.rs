use archon_tui::markdown::render_markdown;
use ratatui::style::{Color, Modifier};

/// Helper: collect all spans from rendered lines, flatten into (text, style) pairs.
fn flat_spans(lines: &[ratatui::text::Line<'static>]) -> Vec<(String, ratatui::style::Style)> {
    lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| (span.content.to_string(), span.style))
        .collect()
}

/// Helper: check if any span contains the given modifier.
fn any_span_has_modifier(
    lines: &[ratatui::text::Line<'static>],
    modifier: Modifier,
) -> bool {
    lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .any(|span| span.style.add_modifier.contains(modifier))
}

/// Helper: check if any span has the given foreground color.
fn any_span_has_fg(lines: &[ratatui::text::Line<'static>], color: Color) -> bool {
    lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .any(|span| span.style.fg == Some(color))
}

#[test]
fn heading_1_styled() {
    let lines = render_markdown("# Title");
    assert!(!lines.is_empty());
    // Heading 1 should be bold + cyan
    assert!(any_span_has_modifier(&lines, Modifier::BOLD));
    assert!(any_span_has_fg(&lines, Color::Cyan));
    let spans = flat_spans(&lines);
    assert!(spans.iter().any(|(text, _)| text.contains("Title")));
}

#[test]
fn heading_2_styled() {
    let lines = render_markdown("## Subtitle");
    assert!(!lines.is_empty());
    assert!(any_span_has_modifier(&lines, Modifier::BOLD));
    assert!(any_span_has_fg(&lines, Color::Blue));
    let spans = flat_spans(&lines);
    assert!(spans.iter().any(|(text, _)| text.contains("Subtitle")));
}

#[test]
fn heading_3_styled() {
    let lines = render_markdown("### Section");
    assert!(!lines.is_empty());
    assert!(any_span_has_modifier(&lines, Modifier::BOLD));
    assert!(any_span_has_fg(&lines, Color::Green));
    let spans = flat_spans(&lines);
    assert!(spans.iter().any(|(text, _)| text.contains("Section")));
}

#[test]
fn bold_text() {
    let lines = render_markdown("**bold**");
    assert!(!lines.is_empty());
    assert!(any_span_has_modifier(&lines, Modifier::BOLD));
    let spans = flat_spans(&lines);
    assert!(spans.iter().any(|(text, _)| text.contains("bold")));
}

#[test]
fn italic_text() {
    let lines = render_markdown("*italic*");
    assert!(!lines.is_empty());
    assert!(any_span_has_modifier(&lines, Modifier::ITALIC));
    let spans = flat_spans(&lines);
    assert!(spans.iter().any(|(text, _)| text.contains("italic")));
}

#[test]
fn inline_code_styled() {
    let lines = render_markdown("some `code` here");
    assert!(!lines.is_empty());
    assert!(any_span_has_fg(&lines, Color::Yellow));
    let spans = flat_spans(&lines);
    assert!(spans.iter().any(|(text, _)| text.contains("code")));
}

#[test]
fn code_block_detected() {
    let input = "```python\nprint('hi')\n```";
    let lines = render_markdown(input);
    assert!(!lines.is_empty());
    // Should contain the code text (plain fallback when no grammar .so)
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(all_text.contains("print"));
}

#[test]
fn unordered_list_prefix() {
    let lines = render_markdown("- item one\n- item two");
    assert!(!lines.is_empty());
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    // Should have bullet prefix and the text
    assert!(all_text.contains('\u{2022}') || all_text.contains('-'));
    assert!(all_text.contains("item one"));
    assert!(all_text.contains("item two"));
}

#[test]
fn ordered_list_prefix() {
    let lines = render_markdown("1. first\n2. second");
    assert!(!lines.is_empty());
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(all_text.contains("1.") || all_text.contains("first"));
    assert!(all_text.contains("second"));
}

#[test]
fn blockquote_styled() {
    let lines = render_markdown("> quoted text");
    assert!(!lines.is_empty());
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(all_text.contains("quoted text"));
    // Should have the blockquote prefix
    assert!(all_text.contains('\u{2502}') || all_text.contains('>'));
}

#[test]
fn link_shows_url() {
    let lines = render_markdown("[click here](https://example.com)");
    assert!(!lines.is_empty());
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(all_text.contains("click here"));
    assert!(all_text.contains("https://example.com"));
}

#[test]
fn empty_input_empty_output() {
    let lines = render_markdown("");
    // Empty input should produce empty or minimal output
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(all_text.trim().is_empty());
}

#[test]
fn table_renders_with_separators() {
    let input = "| Name | Age |\n|------|-----|\n| Alice | 30 |\n| Bob | 25 |";
    let lines = render_markdown(input);
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(
        all_text.contains("Name"),
        "Table should render header cell 'Name', got: {all_text}"
    );
    assert!(
        all_text.contains("Age"),
        "Table should render header cell 'Age', got: {all_text}"
    );
    assert!(
        all_text.contains("Alice"),
        "Table should render data cell 'Alice', got: {all_text}"
    );
    assert!(
        all_text.contains("Bob"),
        "Table should render data cell 'Bob', got: {all_text}"
    );
    // Header cells should be bold
    assert!(
        any_span_has_modifier(&lines, Modifier::BOLD),
        "Table header cells should be bold"
    );
    // Should have pipe separators
    assert!(
        all_text.contains('|') || all_text.contains('\u{2502}'),
        "Table should have column separators, got: {all_text}"
    );
}

#[test]
fn plain_text_passthrough() {
    let lines = render_markdown("just some plain text");
    assert!(!lines.is_empty());
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(all_text.contains("just some plain text"));
}
