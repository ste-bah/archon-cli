//! TASK-AGS-816: /voice slash-command handler (Option C, gap-fix,
//! DIRECT pattern).
//!
//! NEW primary command — `/voice` did NOT exist in shipped slash.rs or
//! registry.rs pre-AGS-816. This is the SECOND Batch-3 Q4=A gap-fix
//! ticket to land a real primary (after AGS-812 `/hooks`). Like
//! `/hooks`, `/voice` is a DIRECT-pattern handler that loads the
//! shipped `ArchonConfig` via `archon_core::config::load_config` at
//! call time and emits the resolved `VoiceConfig` state to the TUI.
//!
//! # Why DIRECT (no snapshot, no effect slot)?
//!
//! `archon_core::config::load_config()` (at
//! `crates/archon-core/src/config.rs:813`) is SYNC (`fn load_config
//! -> Result<ArchonConfig, ConfigError>`, no `.await` anywhere in its
//! body — it just reads and parses a TOML file). So the handler body
//! has no async surface:
//!
//! - NO `VoiceSnapshot` type (nothing to pre-compute inside an async
//!   guard, unlike `/status` / `/model` / `/cost` / `/mcp`).
//! - NO new `CommandContext` field (builder-side drives nothing for
//!   /voice — AGS-822 Rule 5 respected: first ticket that ACTUALLY
//!   needs a context field is the first ticket that adds it, and
//!   /voice does not).
//! - NO `CommandEffect` variant (list subcommand is read-only; write
//!   subcommands — enable / disable / switch — are SCOPE-HELD with a
//!   placeholder TextDelta emit, no actual state mutation).
//!
//! The sole side effect is `ctx.tui_tx.try_send(TuiEvent::TextDelta(..))`
//! — which is sync and legal inside `CommandHandler::execute`. Matches
//! AGS-812 /hooks DIRECT-pattern precedent verbatim.
//!
//! # SCOPE-HELD deferrals
//!
//! Spec `TASK-AGS-816.md` describes a richer surface than shipped
//! infrastructure can support without additional wiring. Per orchestrator
//! decisions (Q1=A sync, Q2=A manual register, Q4=A Option-A minimum
//! viable), the following are SCOPE-HELD and not implemented here:
//!
//! 1. `enable` / `disable` — toggling `VoiceConfig::enabled` requires
//!    persisting `ArchonConfig` back to disk. Shipped `ArchonConfig`
//!    has no ergonomic "save just the voice section" helper; the full
//!    write path is deferred to the first ticket that actually needs
//!    runtime voice-state mutation. Subcommand emits a placeholder
//!    TextDelta directing operators at `~/.archon/settings.json`.
//! 2. `switch` — changing `VoiceConfig::stt_provider` similarly needs
//!    the write path above. Placeholder TextDelta.
//!
//! Each SCOPE-HELD branch emits a uniform
//! `"Voice {sub} command not yet implemented — edit ~/.archon/settings.json directly"`
//! message so operators are informed rather than silently dropped.
//!
//! # Aliases
//!
//! Spec TASK-AGS-816 lists no aliases. Shipped registry has no prior
//! /voice entry. Match registry row: empty alias slice.
//!
//! # Security
//!
//! `VoiceConfig::stt_api_key` may contain a secret bearer token for
//! the STT provider (`openai`, etc.). `render_list` MUST NOT emit the
//! raw key. This handler prints a masked `(set)` / `(empty)` marker
//! instead, which lets operators confirm configuration without leaking
//! credentials into scrollback or TUI snapshots.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

/// Zero-sized handler registered as the primary `/voice` command.
///
/// No aliases. Subcommands dispatched inside `execute`:
/// * `list` (default) — render the resolved `VoiceConfig` via
///   `archon_core::config::load_config`.
/// * `enable` / `disable` / `switch` — SCOPE-HELD placeholder branch.
/// * any other token — unknown-subcommand hint branch.
pub(crate) struct VoiceHandler;

impl CommandHandler for VoiceHandler {
    fn execute(
        &self,
        ctx: &mut CommandContext,
        args: &[String],
    ) -> anyhow::Result<()> {
        // `args.first()` is the first positional token after the command
        // name. Empty args (bare `/voice`) and explicit `list` both map
        // to the list branch. Every other token goes through the
        // three-way match below.
        let sub = args
            .first()
            .map(|s| s.as_str())
            .unwrap_or("list")
            .trim();

        match sub {
            "list" | "" => {
                self.emit_list(ctx);
            }
            "enable" | "disable" | "switch" => {
                let msg = format!(
                    "Voice {sub} command not yet implemented — edit \
                     ~/.archon/settings.json directly"
                );
                let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(msg));
            }
            other => {
                let msg = format!(
                    "Unknown /voice subcommand: {other}. Valid: list, \
                     enable, disable, switch"
                );
                let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(msg));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        "Show or manage voice input configuration"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

impl VoiceHandler {
    /// Render the list output by loading `ArchonConfig` from the
    /// shipped config resolution path and emitting a `TextDelta` with
    /// the resolved `VoiceConfig` fields.
    ///
    /// `archon_core::config::load_config` is sync — it reads the user's
    /// `~/.archon/config.toml` (or the appropriate fallback) and
    /// deserializes via serde. If it fails, the handler emits a
    /// `TuiEvent::Error` rather than panicking, so the TUI surfaces the
    /// failure reason to the operator and returns control cleanly.
    fn emit_list(&self, ctx: &mut CommandContext) {
        let config = match archon_core::config::load_config() {
            Ok(c) => c,
            Err(e) => {
                let _ = ctx.tui_tx.try_send(TuiEvent::Error(format!(
                    "Failed to load config for /voice: {e}"
                )));
                return;
            }
        };
        let text = render_list(&config.voice);
        let _ = ctx.tui_tx.try_send(TuiEvent::TextDelta(text));
    }
}

/// Pure text renderer, factored out for direct unit testing without
/// touching filesystem or TUI channel.
///
/// Security: `stt_api_key` is masked as `(set)` / `(empty)` rather
/// than emitted raw. See module rustdoc.
fn render_list(voice: &archon_core::config::VoiceConfig) -> String {
    let api_key_marker = if voice.stt_api_key.is_empty() {
        "(empty)"
    } else {
        "(set)"
    };
    let mut lines: Vec<String> = Vec::with_capacity(9);
    lines.push("Voice configuration:".to_string());
    lines.push(format!("  enabled:       {}", voice.enabled));
    lines.push(format!("  device:        {}", voice.device));
    lines.push(format!("  stt_provider:  {}", voice.stt_provider));
    lines.push(format!("  stt_api_key:   {api_key_marker}"));
    lines.push(format!("  stt_url:       {}", voice.stt_url));
    lines.push(format!("  vad_threshold: {}", voice.vad_threshold));
    lines.push(format!("  hotkey:        {}", voice.hotkey));
    lines.push(format!("  toggle_mode:   {}", voice.toggle_mode));
    lines.join("\n")
}

// ---------------------------------------------------------------------------
// TASK-AGS-816: tests for /voice slash-command gap-fix
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::config::VoiceConfig;
    use archon_tui::app::TuiEvent;
    use tokio::sync::mpsc;

    /// Build a `CommandContext` with a freshly-created channel.
    /// /voice is a DIRECT-pattern handler — no snapshot, no effect slot
    /// — so every optional field stays `None`. Mirrors the make_ctx
    /// fixtures in hooks.rs / resume.rs / task.rs / cost.rs / model.rs /
    /// status.rs.
    fn make_ctx() -> (CommandContext, mpsc::Receiver<TuiEvent>) {
        let (tx, rx) = mpsc::channel::<TuiEvent>(16);
        (
            CommandContext {
                tui_tx: tx,
                status_snapshot: None,
                model_snapshot: None,
                cost_snapshot: None,
                mcp_snapshot: None,
                context_snapshot: None,
                session_id: None,
                // TASK-AGS-817: /voice tests never exercise /memory paths — None.
                memory: None,
                // TASK-AGS-POST-6-BODIES-B13-GARDEN: /voice tests never exercise /garden paths — None.
                garden_config: None,
                // TASK-AGS-POST-6-BODIES-B01-FAST: /voice tests never exercise /fast paths — None.
                fast_mode_shared: None,
                // TASK-AGS-POST-6-BODIES-B02-THINKING: /voice tests never exercise /thinking paths — None.
                show_thinking: None,
                // TASK-AGS-POST-6-BODIES-B04-DIFF: /voice tests never exercise /diff paths — None.
                working_dir: None,
                // TASK-AGS-POST-6-BODIES-B06-HELP: /voice tests never exercise /help paths — None.
                skill_registry: None,
                // TASK-AGS-POST-6-BODIES-B08-DENIALS: /voice tests never exercise /denials paths — None.
                denial_snapshot: None,
                effort_snapshot: None,
                permissions_snapshot: None,
                pending_effect: None,
                pending_effort_set: None,
            },
            rx,
        )
    }

    #[test]
    fn voice_handler_description_matches() {
        let h = VoiceHandler;
        let desc = h.description().to_lowercase();
        assert!(
            desc.contains("voice"),
            "VoiceHandler description should reference 'voice', got: {}",
            h.description()
        );
    }

    #[test]
    fn voice_handler_has_no_aliases() {
        let h = VoiceHandler;
        assert_eq!(
            h.aliases(),
            &[] as &[&'static str],
            "VoiceHandler must have an empty alias slice per AGS-816 \
             (spec lists no aliases; shipped registry had no prior \
             /voice entry)"
        );
    }

    /// `list` (and the bare no-arg form) must emit a header-bearing
    /// `TextDelta` — OR an `Error` event if the on-disk config fails
    /// to parse. Whatever the operator's config state, the handler
    /// must return `Ok(())` and must not silently drop.
    #[test]
    fn voice_handler_list_emits_textdelta_or_error() {
        let (mut ctx, mut rx) = make_ctx();
        let h = VoiceHandler;
        let res = h.execute(&mut ctx, &["list".to_string()]);
        assert!(
            res.is_ok(),
            "VoiceHandler::execute(list) must return Ok(()), got: {res:?}"
        );

        let mut saw_event = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                TuiEvent::TextDelta(text) => {
                    if text.contains("Voice configuration:") {
                        saw_event = true;
                    }
                }
                TuiEvent::Error(text) => {
                    if text.contains("Failed to load config for /voice") {
                        saw_event = true;
                    }
                }
                _ => {}
            }
        }
        assert!(
            saw_event,
            "VoiceHandler::execute(list) must emit either a \
             'Voice configuration:' TextDelta or a load-failed Error"
        );
    }

    /// Unknown subcommand must emit a friendly one-line hint rather than
    /// silently dropping, panicking, or returning Err(..). The hint
    /// text must enumerate the four valid subcommand tokens so the
    /// operator can pivot without opening the source.
    #[test]
    fn voice_handler_unknown_subcommand_emits_hint() {
        let (mut ctx, mut rx) = make_ctx();
        let h = VoiceHandler;
        let res = h.execute(&mut ctx, &["bogus-sub".to_string()]);
        assert!(
            res.is_ok(),
            "VoiceHandler::execute(bogus-sub) must return Ok(()), \
             got: {res:?}"
        );

        let mut saw_hint = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev {
                if text.contains("Unknown /voice subcommand")
                    && text.contains("list")
                    && text.contains("enable")
                    && text.contains("disable")
                    && text.contains("switch")
                {
                    saw_hint = true;
                }
            }
        }
        assert!(
            saw_hint,
            "VoiceHandler::execute(unknown) must emit a hint TextDelta \
             naming all four valid subcommands"
        );
    }

    /// `enable` / `disable` / `switch` are SCOPE-HELD for AGS-816 —
    /// they emit a placeholder TextDelta pointing at
    /// `~/.archon/settings.json` rather than mutating state. Pin the
    /// placeholder text so a later ticket that upgrades the subcommand
    /// path has to consciously change this test (preventing silent
    /// regressions of the advisory UX).
    #[test]
    fn voice_handler_enable_disable_switch_emit_placeholder() {
        for sub in ["enable", "disable", "switch"] {
            let (mut ctx, mut rx) = make_ctx();
            let h = VoiceHandler;
            let res = h.execute(&mut ctx, &[sub.to_string()]);
            assert!(
                res.is_ok(),
                "VoiceHandler::execute({sub}) must return Ok(()), \
                 got: {res:?}"
            );

            let mut saw_placeholder = false;
            while let Ok(ev) = rx.try_recv() {
                if let TuiEvent::TextDelta(text) = ev {
                    if text.contains(&format!(
                        "Voice {sub} command not yet implemented"
                    )) && text.contains("~/.archon/settings.json")
                    {
                        saw_placeholder = true;
                    }
                }
            }
            assert!(
                saw_placeholder,
                "VoiceHandler::execute({sub}) must emit the canonical \
                 SCOPE-HELD placeholder TextDelta"
            );
        }
    }

    /// Pure `render_list` with a default `VoiceConfig` must produce
    /// the expected header and emit the documented field rows. Guards
    /// the renderer's contract independent of the I/O path exercised
    /// by `voice_handler_list_emits_textdelta_or_error`.
    #[test]
    fn render_list_formats_default_voice_config() {
        let voice = VoiceConfig::default();
        let out = render_list(&voice);
        assert!(
            out.starts_with("Voice configuration:"),
            "render_list must start with 'Voice configuration:', got: {out}"
        );
        // Default VoiceConfig has enabled=false, device="default",
        // stt_provider="openai", hotkey="ctrl+shift+v", toggle_mode=false,
        // vad_threshold=0.02, stt_api_key=String::new().
        assert!(
            out.contains("enabled:       false"),
            "render_list must format enabled field, got: {out}"
        );
        assert!(
            out.contains("device:        default"),
            "render_list must format device field, got: {out}"
        );
        assert!(
            out.contains("stt_provider:  openai"),
            "render_list must format stt_provider field, got: {out}"
        );
        assert!(
            out.contains("hotkey:        ctrl+shift+v"),
            "render_list must format hotkey field, got: {out}"
        );
        assert!(
            out.contains("toggle_mode:   false"),
            "render_list must format toggle_mode field, got: {out}"
        );
        assert!(
            out.contains("vad_threshold: 0.02"),
            "render_list must format vad_threshold field, got: {out}"
        );
    }

    /// Security test: when `stt_api_key` is empty the renderer prints
    /// `(empty)`, and when it is populated the renderer prints `(set)`
    /// — NEVER the raw key. Pins the masking contract so a later edit
    /// cannot accidentally regress to printing the secret.
    #[test]
    fn render_list_masks_stt_api_key() {
        // Empty key -> "(empty)".
        let mut voice = VoiceConfig::default();
        voice.stt_api_key = String::new();
        let out_empty = render_list(&voice);
        assert!(
            out_empty.contains("stt_api_key:   (empty)"),
            "render_list must mask empty stt_api_key as '(empty)', got: {out_empty}"
        );
        assert!(
            !out_empty.contains("sk-"),
            "render_list must never expose a key-like prefix, got: {out_empty}"
        );

        // Populated key -> "(set)", and the raw secret must not appear.
        let secret = "sk-super-secret-should-not-leak-abcdef123456";
        voice.stt_api_key = secret.to_string();
        let out_set = render_list(&voice);
        assert!(
            out_set.contains("stt_api_key:   (set)"),
            "render_list must mask populated stt_api_key as '(set)', got: {out_set}"
        );
        assert!(
            !out_set.contains(secret),
            "render_list must NEVER emit the raw stt_api_key, got: {out_set}"
        );
    }
}
