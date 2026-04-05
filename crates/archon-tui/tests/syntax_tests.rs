use archon_tui::syntax::{CAPTURE_COLORS, grammar_dir, highlight_code, render_plain_code};

/// Helper: flatten all spans from lines.
fn flat_spans(lines: &[ratatui::text::Line<'static>]) -> Vec<(String, ratatui::style::Style)> {
    lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| (span.content.to_string(), span.style))
        .collect()
}

#[test]
fn grammar_dir_under_archon() {
    let dir = grammar_dir();
    let path_str = dir.to_string_lossy();
    assert!(
        path_str.contains("archon") && path_str.contains("grammars"),
        "grammar_dir should contain 'archon/grammars', got: {path_str}"
    );
}

#[test]
fn highlight_unknown_language_returns_none() {
    // No .so grammar file for "brainfuck" will exist
    let result = highlight_code("+++[->+++<]>.", "brainfuck");
    assert!(
        result.is_none(),
        "highlight_code should return None for unknown language without grammar"
    );
}

#[test]
fn render_plain_code_with_language() {
    let lines = render_plain_code("print('hi')", Some("python"));
    assert!(!lines.is_empty());
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    // Should contain the language label
    assert!(
        all_text.contains("python"),
        "plain code with language should show language label"
    );
    // Should contain the code
    assert!(all_text.contains("print"));
}

#[test]
fn render_plain_code_without_language() {
    let lines = render_plain_code("some code here", None);
    assert!(!lines.is_empty());
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(all_text.contains("some code here"));
}

#[test]
fn render_plain_code_multiline() {
    let code = "line one\nline two\nline three";
    let lines = render_plain_code(code, Some("text"));
    // Should have at least 3 lines of code + 1 label line
    assert!(
        lines.len() >= 4,
        "multiline plain code should render at least 4 lines (1 label + 3 code), got {}",
        lines.len()
    );
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(all_text.contains("line one"));
    assert!(all_text.contains("line two"));
    assert!(all_text.contains("line three"));
}

#[test]
fn highlight_captures_mapped() {
    // Verify that CAPTURE_COLORS contains the expected capture name mappings
    let expected = ["keyword", "string", "comment", "function", "type", "number"];
    for name in &expected {
        assert!(
            CAPTURE_COLORS.iter().any(|(k, _)| k == name),
            "CAPTURE_COLORS should contain mapping for '{name}'"
        );
    }
}

#[test]
fn render_plain_code_empty() {
    let lines = render_plain_code("", Some("rust"));
    // Should still produce at least a label line
    assert!(!lines.is_empty());
    let all_text: String = flat_spans(&lines)
        .iter()
        .map(|(t, _)| t.as_str())
        .collect::<Vec<_>>()
        .join("");
    assert!(all_text.contains("rust"));
}
