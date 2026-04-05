//! Configuration source tracking for Archon CLI.
//!
//! Tracks which configuration layer provided each key, enabling the
//! `/config sources` command to show users where each setting originated.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use toml::Value;

use crate::config::ConfigError;
use crate::config_layers::{ConfigLayer, discover_config_paths};

// ---------------------------------------------------------------------------
// ConfigSourceMap
// ---------------------------------------------------------------------------

/// Maps dotted config key paths (e.g. `api.default_model`) to the
/// [`ConfigLayer`] that last set them.
#[derive(Debug, Default)]
pub struct ConfigSourceMap {
    sources: HashMap<String, ConfigLayer>,
}

impl ConfigSourceMap {
    /// Build a source map by walking each discovered config layer.
    ///
    /// Each layer's TOML keys are recorded; later (higher-priority) layers
    /// overwrite earlier entries, so the final map shows which layer "wins"
    /// for every key.
    pub fn from_layered_load(
        user: Option<&Path>,
        work_dir: &Path,
        settings: Option<&Path>,
        filter: Option<&[ConfigLayer]>,
    ) -> Result<Self, ConfigError> {
        let discovered = discover_config_paths(user, work_dir, None);

        let active_layers: Vec<_> = discovered
            .iter()
            .filter(|info| match &filter {
                Some(allowed) => allowed.contains(&info.layer),
                None => true,
            })
            .collect();

        let mut sources = HashMap::new();

        for info in &active_layers {
            let content = match fs::read_to_string(&info.path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let value: Value = match content.parse() {
                Ok(v) => v,
                Err(_) => continue,
            };
            walk_keys(&value, "", &info.layer, &mut sources);
        }

        // Settings overlay
        if let Some(path) = settings
            && path.exists()
            && let Ok(content) = fs::read_to_string(path)
            && let Ok(value) = content.parse::<Value>()
        {
            walk_keys(&value, "", &ConfigLayer::Settings, &mut sources);
        }

        Ok(Self { sources })
    }

    /// Look up which layer provides a given dotted key path.
    pub fn get(&self, key: &str) -> Option<&ConfigLayer> {
        self.sources.get(key)
    }
}

// ---------------------------------------------------------------------------
// Format
// ---------------------------------------------------------------------------

/// Format a source map as human-readable text showing each key and its
/// originating layer. Keys are sorted alphabetically.
///
/// Example output:
/// ```text
///   api.default_model = project
///   api.max_retries = user
/// ```
pub fn format_sources(sources: &ConfigSourceMap) -> String {
    let mut keys: Vec<&String> = sources.sources.keys().collect();
    keys.sort();

    let mut output = String::new();
    for key in keys {
        if let Some(layer) = sources.sources.get(key) {
            output.push_str(&format!("  {key} = {layer}\n"));
        }
    }
    output
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Recursively walk a TOML value tree, recording leaf key paths into the map.
fn walk_keys(
    value: &Value,
    prefix: &str,
    layer: &ConfigLayer,
    map: &mut HashMap<String, ConfigLayer>,
) {
    match value {
        Value::Table(table) => {
            for (key, val) in table {
                let full_key = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                walk_keys(val, &full_key, layer, map);
            }
        }
        // Leaf value (scalar, array, etc.) — record it
        _ => {
            map.insert(prefix.to_string(), layer.clone());
        }
    }
}
