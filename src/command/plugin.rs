//! Plugin management command handler.
//! Extracted from main.rs to reduce main.rs from 6234 to < 500 lines.
//!
//! TASK-#216 SLASH-PLUGIN + TASK-#217 SLASH-RELOAD-PLUGINS extract the
//! plugin-loader construction into [`load_plugins_from_default_dirs`]
//! so the slash handlers (`PluginSlashHandler` in `plugin_slash.rs`,
//! `ReloadPluginsHandler` in `reload_plugins.rs`) can call the same
//! resolver as the CLI surface — avoids drift in plugins-dir / cache-
//! dir / seed-dir resolution between the two surfaces.
//!
//! GHOST-005: Adds plugin enable/disable state persistence to
//! ~/.local/state/archon/plugin-state.json and a
//! `load_plugins_from_default_dirs_with_state` variant that passes
//! enable state through to `PluginLoader::with_enabled_state`.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::cli_args::PluginAction;

/// Build a fresh `PluginLoader` from the default Archon directories
/// (`~/.local/share/archon/plugins`, `~/.cache/archon/wasm`,
/// `ARCHON_PLUGIN_SEED_DIR`) and run `load_all()`. Returns the
/// `PluginLoadResult` directly.
///
/// Shared by the CLI handler (`handle_plugin_command`) and the slash
/// handlers added in TASK-#216 / TASK-#217. No session state, no
/// caching across calls — every invocation re-scans disk + re-parses
/// manifests. This matches the shipped CLI behaviour.
pub(crate) fn load_plugins_from_default_dirs() -> archon_plugin::result::PluginLoadResult {
    use archon_plugin::loader::PluginLoader;

    let plugins_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("archon")
        .join("plugins");

    // Check ARCHON_PLUGIN_SEED_DIR env var
    let seed_dirs: Vec<std::path::PathBuf> = std::env::var("ARCHON_PLUGIN_SEED_DIR")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .collect();

    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".cache"))
        .join("archon")
        .join("wasm");
    let mut loader =
        PluginLoader::new(plugins_dir).with_cache(archon_plugin::cache::WasmCache::new(cache_dir));
    if !seed_dirs.is_empty() {
        loader = loader.with_seed_dirs(seed_dirs);
    }
    loader.load_all()
}

/// GHOST-005: Same as `load_plugins_from_default_dirs` but passes
/// plugin enable/disable state via `PluginLoader::with_enabled_state`.
pub(crate) fn load_plugins_from_default_dirs_with_state(
    state: &HashMap<String, bool>,
) -> archon_plugin::result::PluginLoadResult {
    use archon_plugin::loader::PluginLoader;

    let plugins_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("archon")
        .join("plugins");

    let seed_dirs: Vec<std::path::PathBuf> = std::env::var("ARCHON_PLUGIN_SEED_DIR")
        .unwrap_or_default()
        .split(':')
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .collect();

    let cache_dir = dirs::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".cache"))
        .join("archon")
        .join("wasm");
    let mut loader =
        PluginLoader::new(plugins_dir).with_cache(archon_plugin::cache::WasmCache::new(cache_dir));
    if !seed_dirs.is_empty() {
        loader = loader.with_seed_dirs(seed_dirs);
    }
    loader.with_enabled_state(state.clone()).load_all()
}

// ---------------------------------------------------------------------------
// GHOST-005: plugin enable/disable state persistence
// ---------------------------------------------------------------------------

/// Path to the plugin state JSON file.
fn plugin_state_path() -> std::path::PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("archon")
        .join("plugin-state.json")
}

/// Load persisted plugin enable/disable state from
/// `~/.local/state/archon/plugin-state.json`. Returns an empty map on
/// missing file or parse error (fail-open).
pub(crate) fn load_plugin_enable_state() -> Arc<RwLock<HashMap<String, bool>>> {
    let path = plugin_state_path();
    let state = match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str::<HashMap<String, bool>>(&content).unwrap_or_default(),
        Err(_) => HashMap::new(),
    };
    Arc::new(RwLock::new(state))
}

/// Persist plugin enable/disable state to disk. Creates the parent
/// directory if needed.
pub(crate) fn save_plugin_enable_state(state: &HashMap<String, bool>) -> Result<(), String> {
    let path = plugin_state_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(state).map_err(|e| format!("json: {e}"))?;
    std::fs::write(&path, &json).map_err(|e| format!("write: {e}"))?;
    Ok(())
}

pub fn handle_plugin_command(action: PluginAction) -> anyhow::Result<()> {
    let result = load_plugins_from_default_dirs();

    match action {
        PluginAction::List => {
            println!("{:<30} {:<12} STATUS", "NAME", "VERSION");
            println!("{}", "-".repeat(56));
            for plugin in &result.enabled {
                println!(
                    "{:<30} {:<12} enabled",
                    plugin.manifest.name, plugin.manifest.version
                );
            }
            for plugin in &result.disabled {
                println!(
                    "{:<30} {:<12} disabled",
                    plugin.manifest.name, plugin.manifest.version
                );
            }
            for (id, err) in &result.errors {
                println!("{:<30} {:<12} error: {err}", id, "?");
            }
            if result.enabled.is_empty() && result.disabled.is_empty() && result.errors.is_empty() {
                println!("No plugins found.");
            }
        }
        PluginAction::Info { name } => {
            let plugin = result
                .enabled
                .iter()
                .chain(result.disabled.iter())
                .find(|p| p.manifest.name == name);
            match plugin {
                Some(p) => {
                    let status = if result.disabled.iter().any(|d| d.manifest.name == name) {
                        "disabled"
                    } else {
                        "enabled"
                    };
                    println!("Name:        {}", p.manifest.name);
                    println!("Version:     {}", p.manifest.version);
                    println!("Status:      {status}");
                    if let Some(desc) = &p.manifest.description {
                        println!("Description: {desc}");
                    }
                    if !p.manifest.capabilities.is_empty() {
                        println!("Capabilities: {}", p.manifest.capabilities.join(", "));
                    }
                    println!("Data dir:    {}", p.data_dir.display());
                }
                None => {
                    // Check errors
                    if let Some((_, err)) = result.errors.iter().find(|(id, _)| id == &name) {
                        eprintln!("Plugin '{name}' failed to load: {err}");
                    } else {
                        eprintln!("Plugin '{name}' not found.");
                    }
                }
            }
        }
    }
    Ok(())
}
