use std::fs;
use std::path::PathBuf;

use super::*;

/// Used when creating a new config file on first run.
pub fn write_example_config() -> String {
    include_str!("../../../../config.toml").to_string()
}

/// Load configuration from the default path. If the file does not exist,
/// create the parent directory and write a default config file.
pub fn load_config() -> Result<ArchonConfig, ConfigError> {
    load_config_from(default_config_path())
}

/// Load configuration from a specific path. If the file does not exist,
/// create the parent directory, write a default config, and return defaults.
pub fn load_config_from(path: PathBuf) -> Result<ArchonConfig, ConfigError> {
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, write_example_config())?;
        return Ok(ArchonConfig::default());
    }

    let content = fs::read_to_string(&path)?;
    let config: ArchonConfig = toml::from_str(&content)?;
    validate(&config)?;
    warn_incoherent_permissions(&config, &path);
    Ok(config)
}

/// Load configuration from an existing path without creating a default file.
pub fn load_config_if_exists(path: PathBuf) -> Result<Option<ArchonConfig>, ConfigError> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(&path)?;
    let config: ArchonConfig = toml::from_str(&content)?;
    validate(&config)?;
    warn_incoherent_permissions(&config, &path);
    Ok(Some(config))
}

/// Report permission/sandbox combinations that validate field-by-field and
/// still contradict each other (#200 Phase 3).
///
/// Warnings only. Every config that loaded before this existed still loads,
/// with identical behaviour — the check reads the knobs and writes a log line,
/// and changes no value and no decision.
fn warn_incoherent_permissions(config: &ArchonConfig, path: &std::path::Path) {
    for warning in permission_coherence_warnings(config) {
        tracing::warn!(config = %path.display(), "{warning}");
    }
}

/// Write a named preset's tuple into the HOME config file.
///
/// Same full-rewrite shape as [`save_world_model_guardrail_modes`]: the file is
/// machine-generated from defaults and carries no hand-curated comments worth
/// preserving. The result is validated before it is written, so a preset can
/// never leave an unloadable config behind.
pub fn save_permission_preset(
    name: &str,
) -> Result<(PathBuf, &'static PermissionPreset), ConfigError> {
    let path = default_config_path();
    let mut config = if path.exists() {
        let content = fs::read_to_string(&path)?;
        toml::from_str::<ArchonConfig>(&content)?
    } else {
        ArchonConfig::default()
    };

    let preset = apply_permission_preset(&mut config, name)?;
    validate(&config)?;

    let serialized = toml::to_string_pretty(&config)
        .map_err(|e| ConfigError::ValidationError(format!("TOML serialize error: {e}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serialized)?;
    Ok((path, preset))
}

/// GHOST-008: persist `voice.enabled` to the HOME config file.
///
/// Loads the existing config (or default if file missing), updates
/// `voice.enabled`, serializes back to TOML, and writes to
/// `~/.config/archon/config.toml`. Uses full-rewrite (not surgical
/// TOML edit) — the config file is machine-generated from defaults
/// and does not carry hand-curated comments worth preserving.
pub fn save_voice_enabled(enabled: bool) -> Result<(), ConfigError> {
    let path = default_config_path();
    let mut config = if path.exists() {
        let content = fs::read_to_string(&path)?;
        toml::from_str::<ArchonConfig>(&content)?
    } else {
        ArchonConfig::default()
    };
    config.voice.enabled = enabled;
    let serialized = toml::to_string_pretty(&config)
        .map_err(|e| ConfigError::ValidationError(format!("TOML serialize error: {e}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serialized)?;
    Ok(())
}

/// Persist selected world-model guardrail modes to the HOME config file.
pub fn save_world_model_guardrail_modes(
    interactive_mode: Option<&str>,
    pipeline_mode: Option<&str>,
) -> Result<PathBuf, ConfigError> {
    let path = default_config_path();
    let mut config = if path.exists() {
        let content = fs::read_to_string(&path)?;
        toml::from_str::<ArchonConfig>(&content)?
    } else {
        ArchonConfig::default()
    };
    if let Some(mode) = interactive_mode {
        config.learning.world_model.guardrails.interactive_mode = mode.to_string();
    }
    if let Some(mode) = pipeline_mode {
        config.learning.world_model.guardrails.pipeline_mode = mode.to_string();
    }
    validate(&config)?;
    let serialized = toml::to_string_pretty(&config)
        .map_err(|e| ConfigError::ValidationError(format!("TOML serialize error: {e}")))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, serialized)?;
    Ok(path)
}
