use super::*;

fn render() -> String {
    let handler = crate::command::providers::ProvidersHandler;
    let (mut ctx, mut rx) = crate::command::test_support::make_bug_ctx();
    crate::command::registry::CommandHandler::execute(&handler, &mut ctx, &[]).unwrap();
    let events = crate::command::test_support::drain_tui_events(&mut rx);
    assert_eq!(events.len(), 1);
    match events.into_iter().next().unwrap() {
        archon_tui::app::TuiEvent::TextDelta(text) => text,
        other => panic!("expected TextDelta, got {other:?}"),
    }
}

#[test]
fn execute_emits_total_count_line() {
    let body = render();
    assert!(
        body.contains("37 total: 6 native + 31 openai-compat"),
        "totals line missing or wrong; body:\n{}",
        body
    );
}

#[test]
fn execute_lists_both_section_headers() {
    let body = render();
    assert!(body.contains("NATIVE (6)"), "missing NATIVE header");
    assert!(
        body.contains("OPENAI-COMPAT (31)"),
        "missing OPENAI-COMPAT header"
    );
}

#[test]
fn execute_lists_known_native_providers() {
    let body = render();
    // Spot-check the 6 native providers (GHOST-003: 4 stubs removed; v0.1.40: openai-codex added).
    for id in [
        "openai",
        "anthropic",
        "gemini",
        "xai",
        "bedrock",
        "openai-codex",
    ] {
        assert!(
            body.contains(id),
            "native provider id `{}` missing from output; body:\n{}",
            id,
            body
        );
    }
}

#[test]
fn execute_lists_known_compat_providers() {
    let body = render();
    // Spot-check 8 of the 31 OpenAI-compat providers.
    for id in [
        "ollama",
        "groq",
        "deepseek",
        "openrouter",
        "mistral",
        "perplexity",
        "fireworks",
        "qwen",
    ] {
        assert!(
            body.contains(id),
            "compat provider id `{}` missing from output; body:\n{}",
            id,
            body
        );
    }
}

#[test]
fn execute_total_row_count_matches_registry_size() {
    // Render and count the data rows (lines starting with two
    // spaces and a non-dash, non-`id` character — i.e. provider
    // rows, not the header or divider). Must equal 37 (6 native + 31 compat).
    let body = render();
    let row_count = body
        .lines()
        .filter(|l| l.starts_with("  ") && !l.starts_with("  -") && !l.starts_with("  id "))
        .count();
    assert_eq!(
        row_count, 37,
        "expected exactly 37 provider rows; got {}; body:\n{}",
        row_count, body
    );
}

#[test]
fn fmt_features_renders_compact_csv_or_none() {
    let all = ProviderFeatures {
        streaming: true,
        tool_calling: true,
        vision: true,
        embeddings: true,
        json_mode: true,
    };
    assert_eq!(fmt_features(&all), "stream,tools,vision,embed,json");

    let none = ProviderFeatures {
        streaming: false,
        tool_calling: false,
        vision: false,
        embeddings: false,
        json_mode: false,
    };
    assert_eq!(fmt_features(&none), "(none)");

    let only_stream = ProviderFeatures {
        streaming: true,
        tool_calling: false,
        vision: false,
        embeddings: false,
        json_mode: false,
    };
    assert_eq!(fmt_features(&only_stream), "stream");
}

#[test]
fn truncate_chars_appends_ellipsis_only_when_over() {
    // Short strings unchanged.
    assert_eq!(truncate_chars("hello", 10), "hello");
    // Exact length unchanged.
    assert_eq!(truncate_chars("hello", 5), "hello");
    // Long strings truncated with ellipsis.
    let long = "abcdefghijklmnop"; // 16 chars
    let truncated = truncate_chars(long, 10);
    assert_eq!(truncated.chars().count(), 10);
    assert!(truncated.ends_with('…'));
    // Multi-byte safe.
    assert_eq!(
        truncate_chars("αβγδεζηθικ", 5).chars().count(),
        5,
        "char-count must respect codepoints, not bytes"
    );
}

#[test]
fn execute_does_not_list_stripped_providers() {
    // GHOST-003 stripped the 4 stub native providers (azure, cohere,
    // copilot, minimax) entirely from NATIVE_REGISTRY. They must NOT
    // appear in /providers output at all (no [gap] marker, no row).
    let body = render();
    for id in ["azure", "cohere", "copilot", "minimax"] {
        assert!(
            !body.contains(id),
            "stripped stub provider `{}` must not appear in /providers \
                 output; body:\n{}",
            id,
            body
        );
    }
}
