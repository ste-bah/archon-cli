//! TASK-#210 SLASH-PROVIDERS — `/providers` slash-command handler.
//!
//! Lists every LLM provider registered in the workspace (37 total =
//! 6 native + 31 OpenAI-compatible) by reading the static
//! `archon_llm::providers::{list_native, list_compat}` registries.
//! No session state is touched — both registries are
//! `lazy_static`-style readonly statics, so the handler runs entirely
//! synchronously without populating a `CommandContext` snapshot.
//!
//! GHOST-003: 4 stub native providers (azure, cohere, copilot, minimax)
//! were removed — they returned LlmError::Unsupported with no real wire
//! implementations. The registry now has 6 real native entries.
//!
//! Output is a single `TuiEvent::TextDelta` carrying a two-section
//! aligned table (NATIVE then OPENAI-COMPAT) — matches the
//! `/status` / `/usage` / `/extra-usage` text-delta precedent rather
//! than the `/mcp` overlay pattern (the provider list is static and
//! does not warrant a custom `TuiEvent` variant + TUI overlay).

use anyhow::Result;
use archon_tui::app::TuiEvent;
use chrono::Utc;

use archon_llm::providers::{
    CompatKind, ProviderDescriptor, ProviderFeatures, count_compat, count_native, list_compat,
    list_native, render_capability_table,
};

use crate::cli_args::ProvidersAction;
use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) fn handle_providers(action: Option<ProvidersAction>) -> Result<()> {
    match action.unwrap_or(ProvidersAction::List) {
        ProvidersAction::List => print!("{}", render_provider_registry()),
        ProvidersAction::Capabilities => print!("{}", render_capability_table()),
        ProvidersAction::Doctor => print!("{}", render_provider_doctor()),
    }
    Ok(())
}

/// `/providers` handler — emits a 40-row aligned table of every
/// registered LLM provider.
pub(crate) struct ProvidersHandler;

impl CommandHandler for ProvidersHandler {
    fn execute(&self, ctx: &mut CommandContext, _args: &[String]) -> anyhow::Result<()> {
        let rendered = match _args.first().map(String::as_str) {
            Some("capabilities") | Some("capability") | Some("caps") => render_capability_table(),
            Some("doctor") | Some("diagnose") => render_provider_doctor(),
            Some("list") | None => render_provider_registry(),
            Some(other) => format!(
                "Unknown /providers subcommand `{other}`.\nUsage: /providers [list|capabilities|doctor]\n"
            ),
        };
        ctx.emit(TuiEvent::TextDelta(rendered));
        Ok(())
    }

    fn description(&self) -> &str {
        "List registered LLM providers and capability support"
    }

    fn aliases(&self) -> &'static [&'static str] {
        // No aliases — `list-providers` and `provider` were considered
        // but `/provider` could be confused with a future singular-form
        // command, so the canonical spelling stays alone.
        &[]
    }
}

fn render_provider_registry() -> String {
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
    out
}

fn render_provider_doctor() -> String {
    let path = archon_llm::tokens::credentials_path();
    let credentials_json = std::fs::read_to_string(&path).ok();
    render_provider_doctor_from_json(path.exists(), credentials_json.as_deref(), codex_disabled())
}

fn render_provider_doctor_from_json(
    credentials_file_exists: bool,
    credentials_json: Option<&str>,
    codex_disabled: bool,
) -> String {
    let anthropic = credentials_json
        .and_then(|json| archon_llm::auth::parse_credentials_json(json).ok())
        .map(|creds| credential_status(creds.expires_at.timestamp_millis()));
    let codex = credentials_json
        .and_then(|json| archon_llm::auth::parse_codex_credentials_json(json).ok())
        .map(|creds| credential_status(creds.expires_at.timestamp_millis()));

    let mut out = String::new();
    out.push_str("Provider doctor (local checks only)\n\n");
    out.push_str(&format!(
        "Credentials file: {}\n",
        if credentials_file_exists {
            "present"
        } else {
            "missing"
        }
    ));
    out.push_str(&format!(
        "Anthropic OAuth:  {}\n",
        anthropic.unwrap_or("missing")
    ));
    let codex_status = if codex_disabled {
        "disabled by ARCHON_CODEX_DISABLED"
    } else {
        codex.unwrap_or("missing")
    };
    out.push_str(&format!("Codex OAuth:     {codex_status}\n"));
    out.push('\n');
    out.push_str("Capability source of truth: `archon providers capabilities` or `/providers capabilities`\n");
    out.push_str("Live provider pings: not run by this local doctor. Future `--live` support should remain opt-in.\n");
    out
}

fn credential_status(expires_at_ms: i64) -> &'static str {
    let now_ms = Utc::now().timestamp_millis();
    if expires_at_ms <= now_ms {
        "present but expired"
    } else {
        "present"
    }
}

fn codex_disabled() -> bool {
    std::env::var("ARCHON_CODEX_DISABLED")
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes"))
        .unwrap_or(false)
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
    let mut features = fmt_features(&d.supports);
    if d.is_gap {
        features.push_str(" [gap]");
    }
    format!(
        "  {:<id$}  {:<display$}  {:<model$}  {}\n",
        d.id,
        display,
        model,
        features,
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

    fn render_args(args: &[&str]) -> String {
        let handler = ProvidersHandler;
        let (mut ctx, mut rx) = make_bug_ctx();
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        handler.execute(&mut ctx, &args).unwrap();
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
    fn execute_capabilities_lists_codex_tui_but_not_pipelines() {
        let body = render_args(&["capabilities"]);
        assert!(body.contains("Archon provider capability matrix"));
        assert!(body.contains("| `openai-codex` |"));
        assert!(body.contains("Backs one-shot chat and full TUI sessions"));
        assert!(body.contains("pipelines/subagents are not wired yet"));
    }

    #[test]
    fn cli_handle_capabilities_renders_without_error() {
        handle_providers(Some(ProvidersAction::Capabilities)).expect("capabilities output");
    }

    #[test]
    fn execute_doctor_reports_local_state_without_tokens() {
        let body = render_args(&["doctor"]);
        assert!(body.contains("Provider doctor (local checks only)"));
        assert!(body.contains("Credentials file:"));
        assert!(body.contains("Anthropic OAuth:"));
        assert!(body.contains("Codex OAuth:"));
        assert!(!body.contains("accessToken"));
        assert!(!body.contains("refreshToken"));
    }

    #[test]
    fn render_provider_doctor_from_json_redacts_credentials() {
        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        let json = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "secret-anthropic-access",
                "refreshToken": "secret-anthropic-refresh",
                "expiresAt": future.timestamp_millis(),
                "scopes": ["user:inference"],
                "subscriptionType": "pro"
            },
            "openaiCodexOauth": {
                "accessToken": "secret-codex-access",
                "refreshToken": "secret-codex-refresh",
                "expiresAt": future.timestamp_millis(),
                "accountId": "acct_secret"
            }
        })
        .to_string();

        let body = render_provider_doctor_from_json(true, Some(&json), false);
        assert!(body.contains("Anthropic OAuth:  present"));
        assert!(body.contains("Codex OAuth:     present"));
        assert!(!body.contains("secret-anthropic-access"));
        assert!(!body.contains("secret-codex-access"));
        assert!(!body.contains("acct_secret"));
    }

    #[test]
    fn render_provider_doctor_marks_codex_kill_switch() {
        let body = render_provider_doctor_from_json(false, None, true);
        assert!(body.contains("Codex OAuth:     disabled by ARCHON_CODEX_DISABLED"));
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
        assert!(body.contains("37 total"));
        assert!(body.contains("NATIVE (5)"));
        assert!(body.contains("OPENAI-COMPAT (31)"));
    }
}
