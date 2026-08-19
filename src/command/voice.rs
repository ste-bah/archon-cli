//! GHOST-008: /voice slash-command handler with real persistence.
//!
//! Reads and writes `voice.enabled` to `~/.config/archon/config.toml`
//! via `archon_core::config::load_config` / `save_voice_enabled`.
//!
//! # Subcommand surface (per AGS-816)
//!
//! - `/voice` or `/voice status` — read and display current config
//! - `/voice on` — persist `enabled = true`
//! - `/voice off` — persist `enabled = false`
//! - `/voice list` — alias for status
//!
//! # Security
//!
//! `stt_api_key` is masked as `(set)` / `(empty)` — never emitted raw.

use crate::command::registry::{CommandContext, CommandHandler};
use archon_tui::app::TuiEvent;

pub(crate) struct VoiceHandler;

impl CommandHandler for VoiceHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        let sub = args.first().map(|s| s.as_str()).unwrap_or("status").trim();

        match sub {
            "status" | "list" | "" => {
                let voice = self.emit_status(ctx);
                // Additive, like the other restored screens (#192): the text
                // above is what a `-p` run keeps, and the overlay is dropped
                // there. Only the bare form opens it — `/voice status` is an
                // instruction to print, and an argument form that grew a
                // window would be a change to an existing surface.
                if args.is_empty()
                    && let Some(vad_threshold) = voice
                {
                    ctx.emit(TuiEvent::ShowVoiceCapture { vad_threshold });
                }
            }
            "on" => match archon_core::config::save_voice_enabled(true) {
                Ok(()) => {
                    ctx.emit(TuiEvent::TextDelta(
                        "Voice enabled. Restart required for change to take effect.\n".into(),
                    ));
                }
                Err(e) => {
                    ctx.emit(TuiEvent::Error(format!(
                        "Failed to persist voice config: {e}"
                    )));
                }
            },
            "off" => match archon_core::config::save_voice_enabled(false) {
                Ok(()) => {
                    ctx.emit(TuiEvent::TextDelta(
                        "Voice disabled. Restart required for change to take effect.\n".into(),
                    ));
                }
                Err(e) => {
                    ctx.emit(TuiEvent::Error(format!(
                        "Failed to persist voice config: {e}"
                    )));
                }
            },
            other => {
                let msg =
                    format!("Unknown /voice subcommand: {other}. Valid: status, list, on, off");
                ctx.emit(TuiEvent::TextDelta(msg));
            }
        }
        Ok(())
    }

    fn description(&self) -> &'static str {
        "Show or toggle voice input configuration (status, on, off)"
    }

    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
}

impl VoiceHandler {
    /// Print the configuration, returning the VAD threshold it reported.
    ///
    /// The threshold is handed back rather than re-read because the overlay
    /// has to mark the level the pipeline actually enforces; a second
    /// `load_config` could disagree with the text just printed.
    fn emit_status(&self, ctx: &mut CommandContext) -> Option<f32> {
        let config = match archon_core::config::load_config() {
            Ok(c) => c,
            Err(e) => {
                ctx.emit(TuiEvent::Error(format!(
                    "Failed to load config for /voice: {e}"
                )));
                return None;
            }
        };
        let text = render_status(&config.voice);
        ctx.emit(TuiEvent::TextDelta(text));
        Some(config.voice.vad_threshold)
    }
}

fn render_status(voice: &archon_core::config::VoiceConfig) -> String {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use archon_core::config::VoiceConfig;

    fn make_ctx() -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
        crate::command::test_support::CtxBuilder::new().build()
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
        assert_eq!(h.aliases(), &[] as &[&'static str]);
    }

    #[test]
    fn voice_handler_status_emits_textdelta_or_error() {
        let (mut ctx, mut rx) = make_ctx();
        let h = VoiceHandler;
        let res = h.execute(&mut ctx, &["status".to_string()]);
        assert!(res.is_ok());

        let mut saw_event = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                TuiEvent::TextDelta(text) if text.contains("Voice configuration:") => {
                    saw_event = true;
                }
                TuiEvent::Error(text) if text.contains("Failed to load config for /voice") => {
                    saw_event = true;
                }
                _ => {}
            }
        }
        assert!(saw_event);
    }

    #[test]
    fn voice_handler_bare_invocation_emits_status() {
        let (mut ctx, mut rx) = make_ctx();
        let h = VoiceHandler;
        let res = h.execute(&mut ctx, &[]);
        assert!(res.is_ok());

        let mut saw_status = false;
        while let Ok(ev) = rx.try_recv() {
            match ev {
                TuiEvent::TextDelta(text) if text.contains("Voice configuration:") => {
                    saw_status = true;
                }
                TuiEvent::Error(text) if text.contains("Failed to load config for /voice") => {
                    saw_status = true;
                }
                _ => {}
            }
        }
        assert!(saw_status, "bare /voice must emit status");
    }

    /// Additive, like every other restored screen (#192): the text form is
    /// what `-p` keeps, and the overlay rides alongside it.
    #[test]
    fn bare_voice_opens_the_capture_overlay_as_well_as_printing_status() {
        let (mut ctx, mut rx) = make_ctx();
        VoiceHandler.execute(&mut ctx, &[]).expect("execute");

        let mut saw_text = false;
        let mut saw_overlay = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                TuiEvent::TextDelta(_) | TuiEvent::Error(_) => saw_text = true,
                TuiEvent::ShowVoiceCapture { .. } => saw_overlay = true,
                _ => {}
            }
        }
        assert!(saw_text, "the text form must survive for print mode");
        assert!(saw_overlay, "bare /voice must open the overlay");
    }

    /// A subcommand is an instruction, not a request to browse, so it must not
    /// pop a window — same rule the other restored screens follow.
    #[test]
    fn a_voice_subcommand_opens_no_overlay() {
        for sub in ["status", "list", "on", "off", "bogus"] {
            let (mut ctx, mut rx) = make_ctx();
            VoiceHandler
                .execute(&mut ctx, &[sub.to_string()])
                .expect("execute");
            while let Ok(event) = rx.try_recv() {
                assert!(
                    !matches!(event, TuiEvent::ShowVoiceCapture { .. }),
                    "/voice {sub} opened the overlay"
                );
            }
        }
    }

    #[test]
    fn voice_handler_unknown_subcommand_emits_hint() {
        let (mut ctx, mut rx) = make_ctx();
        let h = VoiceHandler;
        let res = h.execute(&mut ctx, &["bogus".to_string()]);
        assert!(res.is_ok());

        let mut saw_hint = false;
        while let Ok(ev) = rx.try_recv() {
            if let TuiEvent::TextDelta(text) = ev
                && text.contains("Unknown /voice subcommand")
                && text.contains("status")
                && text.contains("on")
                && text.contains("off")
            {
                saw_hint = true;
            }
        }
        assert!(saw_hint);
    }

    #[test]
    fn render_status_formats_default_voice_config() {
        let voice = VoiceConfig::default();
        let out = render_status(&voice);
        assert!(out.starts_with("Voice configuration:"));
        assert!(out.contains("enabled:       false"));
        assert!(out.contains("device:        default"));
        assert!(out.contains("stt_provider:  openai"));
        // The TUI binding is Ctrl+V; the default has to say so (#192).
        assert!(out.contains("hotkey:        ctrl+v"));
        assert!(out.contains("toggle_mode:   false"));
        assert!(out.contains("vad_threshold: 0.02"));
    }

    #[test]
    fn render_status_masks_stt_api_key() {
        let voice = VoiceConfig::default();
        let out_empty = render_status(&voice);
        assert!(out_empty.contains("stt_api_key:   (empty)"));
        assert!(!out_empty.contains("sk-"));

        let secret = "sk-super-secret-should-not-leak-abcdef123456";
        let voice = VoiceConfig {
            stt_api_key: secret.to_string(),
            ..Default::default()
        };
        let out_set = render_status(&voice);
        assert!(out_set.contains("stt_api_key:   (set)"));
        assert!(!out_set.contains(secret));
    }

    /// GHOST-008: /voice on and /voice off emit the restart-required
    /// message or an Error if persistence fails.
    #[test]
    fn voice_handler_on_off_emit_restart_message() {
        for (sub, _expected_enabled) in [("on", "true"), ("off", "false")] {
            let (mut ctx, mut rx) = make_ctx();
            let h = VoiceHandler;
            let res = h.execute(&mut ctx, &[sub.to_string()]);
            assert!(
                res.is_ok(),
                "VoiceHandler::execute({sub}) must return Ok(())"
            );

            let mut saw_event = false;
            while let Ok(ev) = rx.try_recv() {
                match ev {
                    TuiEvent::TextDelta(text) if text.contains("Restart required") => {
                        saw_event = true;
                    }
                    TuiEvent::Error(text) if text.contains("Failed to persist voice config") => {
                        saw_event = true;
                    }
                    _ => {}
                }
            }
            assert!(
                saw_event,
                "VoiceHandler::execute({sub}) must emit restart message or error"
            );
        }
    }
}
