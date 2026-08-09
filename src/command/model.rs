//! TASK-AGS-808: /model slash-command handler (body-migrate target).
//!
//! Real `CommandHandler` impl moved here from the `declare_handler!` stub
//! in `src/command/registry.rs` and the legacy match arm at
//! `src/command/slash.rs:146-180`. The legacy body had TWO sides:
//!
//! * READ side (no args): display the current model by locking
//!   `slash_ctx.model_override_shared` via `.lock().await`.
//! * WRITE side (arg provided): validate the input and overwrite
//!   `*slash_ctx.model_override_shared.lock().await`.
//!
//! `CommandHandler::execute` is SYNC (Q1=A invariant) so NEITHER of those
//! `.await` calls is legal inside `execute`. Two complementary patterns
//! bridge the gap:
//!
//! 1. READ path — per-ticket [`ModelSnapshot`] populated by
//!    `build_command_context` at the dispatch site BEFORE
//!    `Dispatcher::dispatch` (same shape as AGS-807 for `/status`).
//! 2. WRITE path — new [`CommandEffect`] enum
//!    (`crate::command::registry::CommandEffect`). The sync handler
//!    stashes a variant into `CommandContext::pending_effect`; after
//!    dispatch returns, `slash.rs::handle_slash_command` calls
//!    `command::context::apply_effect` which awaits the mutex write.
//!
//! Aliases: `[m, switch-model]` per spec.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandEffect, CommandHandler};
use crate::slash_context::SlashCommandContext;

/// Owned snapshot of the single value the /model READ path needs from
/// shared state. Built at the dispatch site (where `.await` is allowed)
/// and threaded through [`CommandContext`] so the sync handler can
/// consume without holding locks.
///
/// Field is a plain owned `String` — no `Arc`, no `Mutex`, no borrow.
#[derive(Debug, Clone)]
pub(crate) struct ModelSnapshot {
    /// The resolved current model name: the override if non-empty,
    /// otherwise the configured default. Matches the shipped READ body's
    /// `if ov.is_empty() { default_model } else { ov }` selection.
    pub(crate) current_model: String,
    /// `[models.openai-codex]`, so Codex aliases resolve from config.
    ///
    /// Carried on the snapshot for the same reason `current_model` is: the
    /// handler is sync and cannot reach shared state, so anything it needs is
    /// captured at the dispatch site where `.await` is legal.
    pub(crate) codex_models: archon_core::config::OpenAiCodexModelsConfig,
    /// `[models.anthropic]`, for the same reason and by the same route.
    pub(crate) anthropic_models: archon_core::config::AnthropicModelsConfig,
}

/// Build a [`ModelSnapshot`] by awaiting the `model_override_shared`
/// lock in the SAME order and with the SAME selection logic as the
/// shipped READ path at `src/command/slash.rs:146-180`.
///
/// Called from `build_command_context` ONLY when the primary command
/// resolves to `/model` (or one of its aliases `/m` / `/switch-model`).
/// All other commands leave `model_snapshot = None` to avoid unnecessary
/// lock traffic.
pub(crate) async fn build_model_snapshot(slash_ctx: &SlashCommandContext) -> ModelSnapshot {
    let ov = slash_ctx.model_override_shared.lock().await;
    let current_model = if ov.is_empty() {
        slash_ctx.default_model.clone()
    } else {
        ov.clone()
    };
    ModelSnapshot {
        current_model,
        codex_models: slash_ctx.codex_models.clone(),
        anthropic_models: slash_ctx.anthropic_models.clone(),
    }
    // Guard drops here — lock released before return.
}

const CODEX_SHORTCUTS: &[&str] = &["default", "codex", "mini", "opus", "sonnet", "haiku"];
const CODEX_MODEL_IDS: &[&str] = &["gpt-5.5", "gpt-5.4", "gpt-5.4-mini", "gpt-5.3-codex"];

fn snapshot_shortcuts(snap: &ModelSnapshot) -> String {
    if looks_like_codex_model(&snap.current_model) {
        CODEX_SHORTCUTS.join(", ")
    } else if looks_like_anthropic_model(&snap.current_model) {
        archon_tools::validation::KNOWN_SHORTCUTS
            .iter()
            .map(|(shortcut, _)| *shortcut)
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        "provider model ID".into()
    }
}

fn resolve_model_for_snapshot(input: &str, snap: &ModelSnapshot) -> Result<String, String> {
    if looks_like_codex_model(&snap.current_model) {
        resolve_codex_model_name(input, &snap.codex_models)
    } else if looks_like_anthropic_model(&snap.current_model) {
        resolve_anthropic_model_name(input, &snap.anthropic_models)
    } else {
        validate_provider_model_name(input)
    }
}

/// Resolve an Anthropic alias or model ID, reading `[models.anthropic]`.
///
/// The exact defect the Codex resolver above was written to fix, left standing
/// on the other provider: `opus`/`sonnet`/`haiku` came back from
/// `archon_tools::validation::KNOWN_SHORTCUTS`, a compile-time table whose own
/// doc comment says "the canonical source of truth is `ArchonConfig::models`;
/// production code should call `resolve_anthropic_model(alias, &cfg)` instead
/// of reading this constant directly". Nothing did — the resolver had no
/// callers at all. So `/model opus` selected `claude-opus-4-8` while
/// `[models.anthropic] opus` said `claude-opus-5`, and silently, because a
/// valid model ID came back either way.
///
/// Literal IDs still go through `validate_model_name`, which keeps its
/// did-you-mean suggestions for typos. Only the alias arm changes.
fn resolve_anthropic_model_name(
    input: &str,
    cfg: &archon_core::config::AnthropicModelsConfig,
) -> Result<String, String> {
    let resolved = archon_core::config::resolve_anthropic_model(input, cfg);
    // `resolve_anthropic_model` passes unknown input straight through, so a
    // changed value means an alias matched. Anything else is a literal ID or a
    // typo, and validation still owns that decision.
    if resolved != input.trim() {
        return Ok(resolved);
    }
    archon_tools::validation::validate_model_name(input)
}

fn looks_like_codex_model(model: &str) -> bool {
    let lower = model.trim().to_ascii_lowercase();
    lower.starts_with("gpt-5.") || lower == "default" || lower == "codex" || lower == "mini"
}

fn looks_like_anthropic_model(model: &str) -> bool {
    let lower = model.trim().to_ascii_lowercase();
    lower == "opus" || lower == "sonnet" || lower == "haiku" || lower.starts_with("claude-")
}

fn validate_provider_model_name(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Error: Model ID cannot be empty.".into());
    }
    if trimmed.chars().any(char::is_whitespace) {
        return Err(format!(
            "Error: Model ID cannot contain whitespace: {input}"
        ));
    }
    Ok(trimmed.to_string())
}

/// Resolve a Codex alias or model ID, reading `[models.openai-codex]`.
///
/// This used to answer `default`/`opus`/`sonnet` with a literal `"gpt-5.5"`,
/// `codex` with `"gpt-5.3-codex"` and `mini`/`haiku` with `"gpt-5.4-mini"`,
/// ignoring config entirely — so setting `[models.openai-codex] default =
/// "gpt-5.6-sol"` changed what the provider would have used but not what
/// `/model default` selected, because the alias was rewritten to a literal ID
/// here before the provider ever saw it.
///
/// `resolve_codex_model(alias, &cfg)` is the canonical resolver and existed
/// with no callers; this is now its caller. The compile-time fallback table it
/// replaced (`CODEX_KNOWN_SHORTCUTS`) has been deleted rather than left as a
/// second, drifting copy of the same defaults.
///
/// The cross-provider tier names map as `to_alias_map` documents — `opus` and
/// `sonnet` both to the frontier `default`, `haiku` to `mini` — so a tier name
/// and its Codex equivalent cannot drift apart.
fn resolve_codex_model_name(
    input: &str,
    cfg: &archon_core::config::OpenAiCodexModelsConfig,
) -> Result<String, String> {
    let trimmed = input.trim();
    let lower = trimmed.to_ascii_lowercase();
    match lower.as_str() {
        "default" | "opus" | "sonnet" => {
            return Ok(archon_core::config::resolve_codex_model("default", cfg));
        }
        "codex" => return Ok(archon_core::config::resolve_codex_model("codex", cfg)),
        "mini" | "haiku" => return Ok(archon_core::config::resolve_codex_model("mini", cfg)),
        _ => {}
    }

    if CODEX_MODEL_IDS
        .iter()
        .any(|model| lower == model.to_ascii_lowercase())
        || lower.starts_with("gpt-5.")
    {
        return Ok(trimmed.to_string());
    }

    Err(unknown_codex_model_error(input))
}

fn unknown_codex_model_error(input: &str) -> String {
    let mut candidates: Vec<String> = CODEX_SHORTCUTS
        .iter()
        .map(|shortcut| (*shortcut).into())
        .collect();
    candidates.extend(CODEX_MODEL_IDS.iter().map(|id| (*id).to_string()));
    if let Some(suggestion) = closest_candidate(input, &candidates, 2) {
        return format!(
            "Error: Unknown model '{input}'. Did you mean '{suggestion}'?\n\n\
             Valid shortcuts: {shortcuts}\n\
             Valid model IDs: {ids}",
            shortcuts = CODEX_SHORTCUTS.join(", "),
            ids = CODEX_MODEL_IDS.join(", "),
        );
    }

    format!(
        "Error: Unknown model '{input}'.\n\n\
         Valid shortcuts: {shortcuts}\n\
         Valid model IDs: {ids}",
        shortcuts = CODEX_SHORTCUTS.join(", "),
        ids = CODEX_MODEL_IDS.join(", "),
    )
}

fn closest_candidate(input: &str, candidates: &[String], max_distance: usize) -> Option<String> {
    let mut best: Option<(&str, usize)> = None;
    for candidate in candidates {
        let distance = archon_tools::validation::edit_distance(input, candidate);
        if distance <= max_distance {
            match best {
                None => best = Some((candidate, distance)),
                Some((_, best_distance)) if distance < best_distance => {
                    best = Some((candidate, distance));
                }
                _ => {}
            }
        }
    }
    best.map(|(candidate, _)| candidate.to_string())
}

/// Zero-sized handler registered as the primary `/model` command.
/// Aliases: `[m, switch-model]`.
pub(crate) struct ModelHandler;

impl CommandHandler for ModelHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        // Shipped body uses `s.strip_prefix("/model").unwrap_or("").trim()`
        // which reduces to a single free-form trailing string. The
        // dispatcher hands us parser-tokenized `args: &[String]`. Joining
        // with " " and trimming reproduces the shipped selection for the
        // one-token case `/model opus` and is stable for any
        // hypothetical multi-token future (e.g. flags). Whitespace-only
        // rejoin collapses back to the empty string.
        let arg_str = args.join(" ").trim().to_string();
        let snap = ctx.model_snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "ModelHandler invoked without model_snapshot populated \
                 — build_command_context bug"
            )
        })?;

        if arg_str.is_empty() {
            // READ path: the builder must have populated the snapshot
            // when the primary resolved to `/model`. A `None` here
            // indicates a wiring regression (builder bypassed or alias
            // map drifted); surface it as a loud `Err` rather than a
            // user-facing message.

            // Byte-for-byte faithful to shipped READ body at
            // slash.rs:158-162. Output is a TextDelta (no view opened).
            let msg = format!(
                "\nCurrent model: {current}\n\
                 Usage: /model <name>\n\
                 Shortcuts: {shortcuts}\n",
                current = snap.current_model,
                shortcuts = snapshot_shortcuts(snap),
            );
            ctx.emit(TuiEvent::TextDelta(msg));
            return Ok(());
        }

        // WRITE path: validate, then (on Ok) stash the effect + emit
        // ModelChanged + TextDelta. On Err emit TuiEvent::Error and do
        // NOT stash any effect.
        match resolve_model_for_snapshot(&arg_str, snap) {
            Ok(resolved) => {
                // Sync slot-write: the actual `model_override_shared`
                // mutex write is performed by `apply_effect` in
                // `command::context` after dispatch returns. That is
                // where `.await` is legal.
                ctx.pending_effect = Some(CommandEffect::SetModelOverride(resolved.clone()));
                ctx.emit(TuiEvent::ModelChanged(resolved.clone()));
                ctx.emit(TuiEvent::TextDelta(format!(
                    "\nModel switched to {resolved}.\n"
                )));
            }
            Err(msg) => {
                ctx.emit(TuiEvent::Error(msg));
            }
        }
        Ok(())
    }

    fn description(&self) -> &str {
        "Show or switch the active model"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["m", "switch-model"]
    }
}

// ---------------------------------------------------------------------------
// TASK-AGS-808: tests for /model slash-command body-migrate
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
