//! Plugin management command handler.
//! Extracted from main.rs to reduce main.rs from 6234 to < 500 lines.
//!
//! TASK-#216 SLASH-PLUGIN + TASK-#217 SLASH-RELOAD-PLUGINS extract the
//! plugin-loader construction into [`load_plugins_from_default_dirs`]
//! so the slash handlers (`PluginSlashHandler` in `plugin_slash.rs`,
//! `ReloadPluginsHandler` in `reload_plugins.rs`) can call the same
//! resolver as the CLI surface — avoids drift in plugins-dir / cache-
//! dir / seed-dir resolution between the two surfaces.

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
