//! TUI helper functions extracted from src/main.rs (TUI-325)
//!
//! Contains: --list-output-styles, --list-themes, and voice pipeline setup.

use crate::{Cli, Result};

// -- Output style: --list-output-styles (CLI-310) ----------------------------
pub(crate) fn handle_list_output_styles() -> Result<()> {
    use archon_core::output_style::OutputStyleRegistry;
    use archon_core::output_style_loader::load_styles_from_dir;

    let mut reg = OutputStyleRegistry::new();

    if let Some(home) = dirs::home_dir() {
        let new_dir = home.join(".archon").join("output-styles");
        if new_dir.is_dir() {
            for style in load_styles_from_dir(&new_dir) {
                reg.register(style);
            }
        } else {
            let old_dir = home.join(".claude").join("output-styles");
            if old_dir.is_dir() {
                tracing::warn!(
                    "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                    old_dir.display(),
                    new_dir.display()
                );
                for style in load_styles_from_dir(&old_dir) {
                    reg.register(style);
                }
            }
        }
    }

    println!("Available output styles:");
    for name in reg.list() {
        let style = reg.get(&name).unwrap();
        let has_prompt = if style.prompt.is_some() {
            "injects prompt"
        } else {
            "no injection"
        };
        println!("  {:20} {} [{}]", style.name, style.description, has_prompt);
    }
    Ok(())
}

// -- Theme: --list-themes (CLI-315) ------------------------------------------
pub(crate) fn handle_list_themes(
    cli: &Cli,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    use archon_tui::theme::available_themes;
    use archon_tui::theme_registry::detect_system_theme;

    println!("Available themes:");
    for name in available_themes() {
        println!("  {name}");
    }
    println!("  daltonized  (colorblind-friendly)");
    println!("  auto        (system dark/light detection → {:?})", {
        let detected = detect_system_theme();
        let dark_bg = archon_tui::theme::dark_theme().bg;
        if detected.bg == dark_bg {
            "dark"
        } else {
            "light"
        }
    });

    if let Some(theme_name) = cli.theme.as_deref().or(config.tui.theme.as_deref()) {
        let resolved = archon_tui::theme_registry::ThemeRegistry::new().resolve(theme_name);
        println!(
            "\nActive theme: {theme_name}  (bg={:?}, fg={:?})",
            resolved.bg, resolved.fg
        );
    }
    Ok(())
}

// -- Voice pipeline setup ----------------------------------------------------

/// How many level readings may queue between the capture thread and the TUI.
///
/// Small on purpose. Levels are a live view: a stale one is worse than a
/// dropped one, and the capture thread must never wait on a redraw.
const LEVEL_QUEUE: usize = 8;

/// Start the speech loop, returning the channel replies go into.
///
/// Independent of [`setup_voice_pipeline`]: listening and speaking are separate
/// devices and separate wants, and a machine with no microphone can still read
/// answers aloud.
///
/// Returns `None` — loudly — when speech cannot work, rather than accepting
/// replies into a channel nothing drains.
pub(crate) fn setup_speech(
    config: &archon_core::config::ArchonConfig,
) -> Option<tokio::sync::mpsc::Sender<String>> {
    use archon_tui::voice::speech::{SpeechPipeline, set_speech_enabled, speech_loop};
    use archon_tui::voice::tts::{OpenAiCompatibleTts, TtsProvider};
    use std::sync::Arc as StdArc;

    if !config.voice.speak {
        // Still record the setting: Ctrl+P reads and flips the same flag, so
        // the key works from a known state rather than whatever it defaulted to.
        set_speech_enabled(false);
        return None;
    }

    let sink = match archon_tui::voice::real_speech_player() {
        Ok(sink) => sink,
        Err(error) => {
            tracing::error!("voice: {error:#}");
            eprintln!("warning: speech is enabled but unavailable: {error:#}");
            set_speech_enabled(false);
            return None;
        }
    };

    let tts: StdArc<dyn TtsProvider> = StdArc::new(OpenAiCompatibleTts {
        url: config.voice.tts_url.clone(),
        api_key: config.voice.tts_api_key.clone(),
        model: config.voice.tts_model.clone(),
        voice: config.voice.tts_voice.clone(),
    });

    // Bounded and small. Replies are spoken one at a time and a backlog of
    // them is worse than dropping one: by the time a queued sentence is read
    // out, the conversation has moved on.
    let (text_tx, text_rx) = tokio::sync::mpsc::channel::<String>(4);
    archon_observability::spawn_named("voice-speech", async move {
        speech_loop(text_rx, SpeechPipeline::new(tts, sink)).await;
    });
    set_speech_enabled(true);
    tracing::info!(
        "voice: speech wired (provider={}, voice={}, url={})",
        config.voice.tts_provider,
        config.voice.tts_voice,
        config.voice.tts_url,
    );
    Some(text_tx)
}

/// Start the voice pipeline, or explain why there isn't one.
///
/// Returns `None` — with the reason logged and printed — whenever voice cannot
/// actually work. It used to return a pipeline built on a `MockAudioSource`
/// seeded with a second of zeroes, in *both* branches of a check that logged
/// "real audio device detected". So voice input never failed and never worked:
/// it recorded silence, the VAD discarded it, and nothing anywhere said so.
/// A subsystem that cannot run has to say it cannot run (#192).
pub(crate) async fn setup_voice_pipeline(
    config: &archon_core::config::ArchonConfig,
) -> Option<tokio::sync::mpsc::Receiver<archon_tui::app::TuiEvent>> {
    if !config.voice.enabled {
        tracing::info!("voice: disabled (config.voice.enabled=false)");
        return None;
    }
    use archon_tui::app::TuiEvent as VTuiEvent;
    use archon_tui::voice::pipeline::{
        VoicePipeline, VoiceTrigger, hotkey_action_for_mode, install_toggle_mode,
        install_trigger_sender, voice_loop,
    };
    use archon_tui::voice::stt::{LocalStt, MockStt, OpenAiStt, SttProvider};
    use std::sync::Arc as StdArc;

    let (voice_evt_tx, voice_evt_rx) = tokio::sync::mpsc::channel::<VTuiEvent>(16);
    let (level_tx, mut level_rx) = tokio::sync::mpsc::channel::<f32>(LEVEL_QUEUE);

    let audio = match archon_tui::voice::real_audio_source(&config.voice.device, Some(level_tx)) {
        Ok(audio) => audio,
        Err(error) => {
            // Loud, and on stderr as well as in the log: the user turned voice
            // on and is entitled to know it did not come up.
            tracing::error!("voice: {error:#}");
            eprintln!("warning: voice is enabled but unavailable: {error:#}");
            return None;
        }
    };

    let (trig_tx, trig_rx) = tokio::sync::mpsc::channel::<VoiceTrigger>(16);
    install_trigger_sender(trig_tx);
    install_toggle_mode(config.voice.toggle_mode);
    tracing::info!(
        "voice: toggle_mode={} (hotkey action={:?})",
        config.voice.toggle_mode,
        hotkey_action_for_mode(config.voice.toggle_mode)
    );

    let stt: StdArc<dyn SttProvider> = match config.voice.stt_provider.as_str() {
        "openai" if !config.voice.stt_api_key.is_empty() => StdArc::new(OpenAiStt {
            api_key: config.voice.stt_api_key.clone(),
            url: config.voice.stt_url.clone(),
        }),
        "local" => StdArc::new(LocalStt {
            url: config.voice.stt_url.clone(),
        }),
        other => {
            tracing::warn!(
                "voice: stt_provider={other:?} has no credentials or no implementation; \
                 recordings will transcribe to a placeholder"
            );
            StdArc::new(MockStt {
                response: "[voice: no STT configured]".to_string(),
            })
        }
    };

    // Levels reach the overlay on the same channel as the rest of the voice
    // events, so ordering with VoiceRecording is preserved. try_send, not
    // send: a level that cannot be delivered now is worthless later, and this
    // task must not become backpressure on the microphone.
    let level_events = voice_evt_tx.clone();
    archon_observability::spawn_named("voice-level-meter", async move {
        while let Some(level) = level_rx.recv().await {
            let _ = level_events.try_send(VTuiEvent::VoiceLevel(level));
        }
    });

    let pipeline = VoicePipeline::new(audio, stt, config.voice.vad_threshold);
    archon_observability::spawn_named("voice-pipeline", async move {
        voice_loop(trig_rx, voice_evt_tx, pipeline).await;
    });
    tracing::info!(
        "voice: pipeline wired (provider={}, device={}, hotkey={})",
        config.voice.stt_provider,
        config.voice.device,
        config.voice.hotkey,
    );
    // Give the spawned voice_loop task a chance to emit its startup log.
    tokio::task::yield_now().await;
    Some(voice_evt_rx)
}
