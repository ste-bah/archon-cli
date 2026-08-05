use super::*;

#[test]
fn truncate_content_caps_long_memory_body() {
    let content = "x".repeat(BRIEFING_MEMORY_MAX_CHARS + 100);
    let truncated = truncate_content(&content, BRIEFING_MEMORY_MAX_CHARS);

    assert!(truncated.len() <= BRIEFING_MEMORY_MAX_CHARS + 3);
    assert!(truncated.ends_with("..."));
}

#[test]
fn cap_briefing_preserves_closing_tag() {
    let body = format!(
        "<memory_briefing>\n{}\n</memory_briefing>",
        "x".repeat(BRIEFING_TOTAL_MAX_CHARS * 2)
    );
    let capped = cap_briefing(body);

    assert!(capped.len() <= BRIEFING_TOTAL_MAX_CHARS);
    assert!(capped.contains("[briefing truncated]"));
    assert!(capped.ends_with("</memory_briefing>"));
}
