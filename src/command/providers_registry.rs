//! Static provider registry rendering for `providers`.

use archon_llm::providers::{
    CompatKind, ProviderDescriptor, ProviderFeatures, count_compat, count_native, list_compat,
    list_native,
};

pub(crate) fn render_provider_registry() -> String {
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
#[path = "providers_registry_tests.rs"]
mod tests;
