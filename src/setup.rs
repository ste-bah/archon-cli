// Setup and initialization functions extracted from main.rs
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;

use archon_core::config::default_config_path;
use archon_core::config_layers::ConfigLayer;
use archon_core::env_vars::{self, ArchonEnvVars};
use archon_core::logging::{default_log_dir, init_logging, rotate_logs};
use archon_llm::identity::IdentityProvider;
use archon_tui::app::TuiEvent;

use crate::cli_args::Cli;
use archon_core::cli_flags::ResolvedFlags;

/// Parse `--setting-sources` names into [`ConfigLayer`] variants, warning on
/// unrecognised values.
pub fn parse_layer_filter(sources: &[String]) -> Vec<ConfigLayer> {
    sources
        .iter()
        .filter_map(|s| match s.as_str() {
            "user" => Some(ConfigLayer::User),
            "project" => Some(ConfigLayer::Project),
            "local" => Some(ConfigLayer::Local),
            other => {
                eprintln!("warning: unknown setting source: {other}");
                None
            }
        })
        .collect()
}

/// Strip `cache_control` keys from system prompt blocks when prompt caching
/// is disabled via `config.context.prompt_cache = false` (TASK-WIRE-003).
/// A no-op when `prompt_cache_enabled` is true.
pub fn strip_cache_control_if_disabled(
    blocks: &mut [serde_json::Value],
    prompt_cache_enabled: bool,
) {
    if prompt_cache_enabled {
        return;
    }
    for block in blocks.iter_mut() {
        if let Some(obj) = block.as_object_mut() {
            obj.remove("cache_control");
        }
    }
}

/// Initialize logging system and return the log directory.
/// The log guard is stored internally and will be dropped when the function returns,
/// but that's acceptable since the logging system is already initialized.
pub fn setup_logging(session_id: &str, log_level: &str) -> Result<PathBuf> {
    let log_dir = std::env::var_os("ARCHON_LOG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(default_log_dir);

    init_logging(session_id, log_level, &log_dir).map_err(|e| {
        anyhow::anyhow!("logging init failed: {e}")
    })?;

    Ok(log_dir)
}

/// Resolve CLI flags and apply them to config (model override, log level, etc.).
/// Returns the resolved flags for later use.
pub fn resolve_cli_flags(
    cli: &Cli,
    config: &mut archon_core::config::ArchonConfig,
) -> archon_core::cli_flags::ResolvedFlags {
    use archon_core::cli_flags::resolve_flags;

    let resolved_flags = resolve_flags(&cli.to_flag_input()).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    // --model overrides config default model (higher priority than env var)
    if let Some(ref model) = resolved_flags.model {
        config.api.default_model = model.clone();
    }

    // --verbose bumps logging to trace
    if resolved_flags.verbose {
        config.logging.level = "trace".to_string();
    }

    // --debug sets debug-level logging with optional category filter
    if let Some(ref filter) = resolved_flags.debug {
        match filter {
            Some(categories) => {
                config.logging.level = format!(
                    "warn,{}",
                    categories
                        .split(',')
                        .map(|c| format!("{c}=debug"))
                        .collect::<Vec<_>>()
                        .join(",")
                );
            }
            None => {
                config.logging.level = "debug".to_string();
            }
        }
    }

    // ARCHON_LOG env var overrides log level (e.g. ARCHON_LOG=debug)
    if let Ok(log_level) = std::env::var("ARCHON_LOG")
        && !log_level.trim().is_empty()
    {
        config.logging.level = log_level.trim().to_string();
    }

    resolved_flags
}

/// Set up voice pipeline if enabled in config.
/// Returns the voice event receiver if voice is enabled.
pub fn setup_voice_pipeline(
    config: &archon_core::config::ArchonConfig,
) -> Option<tokio::sync::mpsc::Receiver<TuiEvent>> {
    if !config.voice.enabled {
        tracing::info!("voice: disabled (config.voice.enabled=false)");
        return None;
    }

    use archon_tui::app::TuiEvent as VTuiEvent;
    use archon_tui::voice::pipeline::{
        AudioSource, MockAudioSource, VoicePipeline, VoiceTrigger, hotkey_action_for_mode,
        install_toggle_mode, install_trigger_sender, voice_loop,
    };
    use archon_tui::voice::stt::{LocalStt, MockStt, OpenAiStt, SttProvider};
    use std::sync::Arc as StdArc;

    let (trig_tx, trig_rx) = tokio::sync::mpsc::channel::<VoiceTrigger>(16);
    install_trigger_sender(trig_tx);
    install_toggle_mode(config.voice.toggle_mode);
    tracing::info!(
        "voice: toggle_mode={} (hotkey action={:?})",
        config.voice.toggle_mode,
        hotkey_action_for_mode(config.voice.toggle_mode)
    );
    let (voice_evt_tx, voice_evt_rx_inner) = tokio::sync::mpsc::channel::<VTuiEvent>(16);
    let voice_event_rx = Some(voice_evt_rx_inner);
    let audio_capture = archon_tui::voice::capture::AudioCapture::new();
    let audio: StdArc<dyn AudioSource> = if audio_capture.is_supported() {
        tracing::info!(
            "voice: real audio device detected (sample_rate={}, channels={})",
            audio_capture.sample_rate,
            audio_capture.channels
        );
        StdArc::new(MockAudioSource::with_samples(vec![
            0.0_f32;
            audio_capture.sample_rate as usize
        ]))
    } else {
        tracing::warn!("voice: no audio device available, using mock audio source");
        StdArc::new(MockAudioSource::with_samples(vec![0.0_f32; 16000]))
    };
    let stt: StdArc<dyn SttProvider> = match config.voice.stt_provider.as_str() {
        "openai" if !config.voice.stt_api_key.is_empty() => StdArc::new(OpenAiStt {
            api_key: config.voice.stt_api_key.clone(),
            url: config.voice.stt_url.clone(),
        }),
        "local" => StdArc::new(LocalStt {
            url: config.voice.stt_url.clone(),
        }),
        _ => StdArc::new(MockStt {
            response: "[voice: no STT configured]".to_string(),
        }),
    };
    let pipeline = VoicePipeline::new(audio, stt, config.voice.vad_threshold);
    tokio::spawn(async move {
        voice_loop(trig_rx, voice_evt_tx, pipeline).await;
    });
    tracing::info!(
        "voice: pipeline wired (provider={}, device={}, hotkey={})",
        config.voice.stt_provider,
        config.voice.device,
        config.voice.hotkey,
    );

    voice_event_rx
}

/// Load environment variables and warn about unrecognized ARCHON_* vars.
pub fn load_env_vars() -> ArchonEnvVars {
    let env_vars = env_vars::load_env_vars();
    let all_env: std::collections::HashMap<String, String> = std::env::vars().collect();
    let unrecognized = env_vars::warn_unrecognized_archon_vars(&all_env);
    for var_name in &unrecognized {
        eprintln!("warning: unrecognized environment variable: {var_name}");
    }
    env_vars
}

/// Load and merge config from file, CLI settings, and environment overrides.
pub fn load_config(
    env_vars: &ArchonEnvVars,
    cli: &Cli,
) -> (archon_core::config::ArchonConfig, std::path::PathBuf) {
    let config_path = env_vars
        .config_dir
        .as_ref()
        .map(|d| d.join("config.toml"))
        .unwrap_or_else(default_config_path);

    let layer_filter: Option<Vec<ConfigLayer>> =
        cli.setting_sources.as_ref().map(|s| parse_layer_filter(s));

    let working_dir_for_config = std::env::current_dir().unwrap_or_default();
    let mut config = archon_core::config_layers::load_layered_config(
        Some(&config_path),
        &working_dir_for_config,
        cli.settings.as_deref(),
        layer_filter.as_deref(),
    )
    .unwrap_or_else(|e| {
        eprintln!("warning: failed to load config, using defaults: {e}");
        archon_core::config::ArchonConfig::default()
    });

    // Apply env var overrides on top of config file
    env_vars::apply_env_overrides(&mut config, env_vars);

    (config, config_path)
}

/// Log startup information about memory and prompt cache settings.
pub fn log_startup_info(config: &archon_core::config::ArchonConfig, session_id: &str) {
    tracing::info!("Archon CLI v0.1.0 started, session {session_id}");
    if config.memory.enabled {
        tracing::info!("memory.enabled=true: memory tools + graph injection ACTIVE");
    } else {
        tracing::info!("memory.enabled=false: memory tools and graph injection DISABLED");
    }
    if config.context.prompt_cache {
        tracing::info!("context.prompt_cache=true: cache_control hints ACTIVE");
    } else {
        tracing::info!("context.prompt_cache=false: cache_control hints DISABLED");
    }
    tracing::debug!(
        "context: compact_threshold={}, max_tokens={:?}",
        config.context.compact_threshold,
        config.context.max_tokens,
    );
}

/// Generate a new session ID.
pub fn generate_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
