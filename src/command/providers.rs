//! TASK-#210 SLASH-PROVIDERS — `/providers` slash-command handler.
//!
//! Lists every LLM provider registered in the workspace (36 total =
//! 5 native + 31 OpenAI-compatible) by reading the static
//! `archon_llm::providers::{list_native, list_compat}` registries.
//! No session state is touched — both registries are
//! `lazy_static`-style readonly statics, so the handler runs entirely
//! synchronously without populating a `CommandContext` snapshot.
//!
//! GHOST-003: 4 stub native providers (azure, cohere, copilot, minimax)
//! were removed — they returned LlmError::Unsupported with no real wire
//! implementations. The registry now has 5 real native entries.
//!
//! Output is a single `TuiEvent::TextDelta` carrying a two-section
//! aligned table (NATIVE then OPENAI-COMPAT) — matches the
//! `/status` / `/usage` / `/extra-usage` text-delta precedent rather
//! than the `/mcp` overlay pattern (the provider list is static and
//! does not warrant a custom `TuiEvent` variant + TUI overlay).

use archon_tui::app::TuiEvent;

use archon_llm::providers::{
    CompatKind, ProviderDescriptor, ProviderFeatures, count_compat, count_native, list_compat,
    list_native,
};

use crate::command::registry::{CommandContext, CommandHandler};

/// `/providers` handler — emits a 40-row aligned table of every
/// registered LLM provider.
pub(crate) struct ProvidersHandler;

impl CommandHandler for ProvidersHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let native = list_native();
        let compat = list_compat();
        let total = count_native() + count_compat();

        let mut out = String::with_capacity(4096);
        out.push('\n');
        out.push_str(&format!(
            "LLM provider registry ({total} total: {n_native} native + {n_compat} openai-compat)\n",
            total = total,
            n_native = count_native(),
            n_compat = count_compat(),
        ));
        out.push('\n');

        // NATIVE section.
        out.push_str(&format!("NATIVE ({})\n", count_native()));
        out.push_str(&header_row());
        out.push_str(&divider_row());
        for d in &native {
            debug_assert_eq!(d.compat_kind, CompatKind::Native);
            out.push_str(&fmt_provider_row(d));
        }
        out.push('\n');

        // OPENAI-COMPAT section.
        out.push_str(&format!("OPENAI-COMPAT ({})\n", count_compat()));
        out.push_str(&header_row());
        out.push_str(&divider_row());
        for d in &compat {
            debug_assert_eq!(d.compat_kind, CompatKind::OpenAiCompat);
            out.push_str(&fmt_provider_row(d));
        }
        out.push('\n');

        out.push_str(
            "Tip: configure a provider in [llm.<id>] in archon.toml; switch the active\n\
             model with /model <name>.\n",
        );

        ctx.emit(TuiEvent::TextDelta(out));
        Ok(())
    }

    fn description(&self) -> &str {
        "List every registered LLM provider (native + OpenAI-compatible)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // No aliases — `list-providers` and `provider` were considered
        // but `/provider` could be confused with a future singular-form
        // command, so the canonical spelling stays alone.
        &[]
    }
}

// Column widths kept in module-private constants so the header,
// divider, and data rows stay in lockstep — change one, change all.
const COL_ID: usize = 15;
const COL_DISPLAY: usize = 20;
const COL_MODEL: usize = 36;

fn header_row() -> String {
    format!(
        "  {:<id$}  {:<display$}  {:<model$}  features\n",
        "id",
        "display name",
        "default model",
        id = COL_ID,
        display = COL_DISPLAY,
        model = COL_MODEL,
    )
}

fn divider_row() -> String {
    let make = |n: usize| "-".repeat(n);
    format!(
        "  {}  {}  {}  {}\n",
        make(COL_ID),
        make(COL_DISPLAY),
        make(COL_MODEL),
        make(8),
    )
}

fn fmt_provider_row(d: &ProviderDescriptor) -> String {
    let display = truncate_chars(&d.display_name, COL_DISPLAY);
    let model = truncate_chars(&d.default_model, COL_MODEL);
    format!(
        "  {:<id$}  {:<display$}  {:<model$}  {}\n",
        d.id,
        display,
        model,
        fmt_features(&d.supports),
        id = COL_ID,
        display = COL_DISPLAY,
        model = COL_MODEL,
    )
}

/// Truncate a `&str` to at most `max` Unicode characters, appending
/// `…` when shortened. Char-aware — never panics on multi-byte input.
fn truncate_chars(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        return s.to_string();
    }
    // max - 1 chars plus a `…` (1 char) keeps total <= max.
    let take = max.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
}

fn fmt_features(f: &ProviderFeatures) -> String {
    let mut parts: Vec<&'static str> = Vec::with_capacity(5);
    if f.streaming {
        parts.push("stream");
    }
    if f.tool_calling {
        parts.push("tools");
    }
    if f.vision {
        parts.push("vision");
    }
    if f.embeddings {
        parts.push("embed");
    }
    if f.json_mode {
        parts.push("json");
    }
    if parts.is_empty() {
        "(none)".to_string()
    } else {
        parts.join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::test_support::*;

    fn render() -> String {
        let handler = ProvidersHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        match events.into_iter().next().unwrap() {
            TuiEvent::TextDelta(s) => s,
            other => panic!("expected TextDelta, got {:?}", other),
        }
    }

    #[test]
    fn execute_emits_total_count_line() {
        let body = render();
        assert!(
            body.contains("36 total: 5 native + 31 openai-compat"),
            "totals line missing or wrong; body:\n{}",
            body
        );
    }

    #[test]
    fn execute_lists_both_section_headers() {
        let body = render();
        assert!(body.contains("NATIVE (5)"), "missing NATIVE header");
        assert!(
            body.contains("OPENAI-COMPAT (31)"),
            "missing OPENAI-COMPAT header"
        );
    }

    #[test]
    fn execute_lists_known_native_providers() {
        let body = render();
        // Spot-check the 5 native providers (GHOST-003: 4 stubs removed).
        for id in ["openai", "anthropic", "gemini", "xai", "bedrock"] {
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
        // rows, not the header or divider). Must equal 40.
        let body = render();
        let row_count = body
            .lines()
            .filter(|l| l.starts_with("  ") && !l.starts_with("  -") && !l.starts_with("  id "))
            .count();
        assert_eq!(
            row_count, 36,
            "expected exactly 36 provider rows; got {}; body:\n{}",
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
    fn description_and_aliases() {
        let h = ProvidersHandler;
        assert!(!h.description().is_empty());
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    #[ignore = "Gate 5 live smoke — exercises Registry dispatch via default_registry(), run via --ignored"]
    fn providers_dispatches_via_registry() {
        // Gate 5 smoke: Registry::get("providers") must return Some,
        // and execute must emit a single TextDelta with both section
        // headers + the 40-total marker.
        use crate::command::registry::default_registry;

        let registry = default_registry();
        let handler = registry
            .get("providers")
            .expect("providers must be registered in default_registry()");

        let (mut ctx, mut rx) = make_bug_ctx();
        handler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        assert_eq!(events.len(), 1);
        let body = match &events[0] {
            TuiEvent::TextDelta(s) => s.clone(),
            other => panic!("expected TextDelta, got {:?}", other),
        };
        assert!(body.contains("36 total"));
        assert!(body.contains("NATIVE (5)"));
        assert!(body.contains("OPENAI-COMPAT (31)"));
    }
}
