use std::path::Path;

use tracing::warn;

use super::AgentLoadError;
use super::six_file::load_single_agent;
use crate::agents::definition::{AgentSource, CustomAgentDefinition};

pub fn load_custom_agents(
    dir: &Path,
    source: AgentSource,
) -> Result<Vec<CustomAgentDefinition>, AgentLoadError> {
    let entries = std::fs::read_dir(dir).map_err(|e| AgentLoadError::ReadDir {
        path: dir.display().to_string(),
        source: e,
    })?;

    let mut agents = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip template/schema/archived directories
        if name.starts_with('_') {
            continue;
        }

        agents.push(load_single_agent(&path, &name, source.clone()));
    }

    Ok(agents)
}

/// Load all plugin-provided agents from `plugins_root`.
///
/// Scans `<plugins_root>/<plugin_name>/agents/<agent_name>/` for the 6-file
/// agent structure. Plugin directories whose name starts with `_` are
/// skipped (same convention as [`load_custom_agents`]). Agent subdirectories
/// whose name starts with `_` are also skipped. Loaded agents have their
/// `agent_type` prefixed with `<plugin_name>:` and carry
/// [`AgentSource::Plugin`]`(plugin_name)` as provenance.
///
/// A non-existent `plugins_root` returns `Ok(vec![])` — plugin roots are
/// optional, not an error condition. A plugin directory without an
/// `agents/` subdir is silently skipped (the plugin may provide only
/// skills, hooks, or other resources).
pub fn load_plugin_agents(
    plugins_root: &Path,
) -> Result<Vec<CustomAgentDefinition>, AgentLoadError> {
    if !plugins_root.is_dir() {
        return Ok(Vec::new());
    }

    let entries = std::fs::read_dir(plugins_root).map_err(|e| AgentLoadError::ReadDir {
        path: plugins_root.display().to_string(),
        source: e,
    })?;

    let mut agents = Vec::new();

    for entry in entries.flatten() {
        let plugin_path = entry.path();
        if !plugin_path.is_dir() {
            continue;
        }
        let plugin_name = match plugin_path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        // Skip private/template/archived plugin dirs (same rule as custom agents).
        if plugin_name.starts_with('_') {
            continue;
        }

        // Agents live at <plugin>/agents/<agent_name>/
        let agents_dir = plugin_path.join("agents");
        if !agents_dir.is_dir() {
            // Plugin without an agents/ subdir: plugin may provide only
            // skills/hooks/commands/etc. Not an error.
            continue;
        }

        let agent_entries = match std::fs::read_dir(&agents_dir) {
            Ok(e) => e,
            Err(e) => {
                warn!(
                    plugin = plugin_name,
                    dir = %agents_dir.display(),
                    error = %e,
                    "failed to read plugin agents dir; skipping plugin"
                );
                continue;
            }
        };

        for agent_entry in agent_entries.flatten() {
            let agent_path = agent_entry.path();
            if !agent_path.is_dir() {
                continue;
            }
            let agent_name = match agent_path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_string(),
                None => continue,
            };
            // Inherit the _-prefix skip rule for agent subdirs too.
            if agent_name.starts_with('_') {
                continue;
            }

            let mut def = load_single_agent(
                &agent_path,
                &agent_name,
                AgentSource::Plugin(plugin_name.clone()),
            );
            // Namespace the agent_type with the plugin name so it can't
            // collide with bare custom agents of the same local name.
            def.agent_type = format!("{plugin_name}:{agent_name}");
            agents.push(def);
        }
    }

    Ok(agents)
}
