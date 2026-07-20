use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use super::HookRegistry;
use crate::hooks::toml_loader;
use crate::hooks::types::{HookError, HookEvent, HookMatcher};

#[derive(Deserialize, Default)]
pub(super) struct SettingsJson {
    #[serde(default)]
    hooks: HashMap<HookEvent, Vec<HookMatcher>>,
}

impl HookRegistry {
    /// Parse `.archon/settings.json` and populate from `"hooks"` field.
    /// Returns `Err` only on JSON parse failure.
    pub fn load_from_settings_json(json: &str) -> Result<Self, HookError> {
        let settings: SettingsJson = serde_json::from_str(json)
            .map_err(|e| HookError::JsonError(format!("settings.json parse error: {e}")))?;

        let registry = Self::new();
        for (event, matchers) in settings.hooks {
            registry.register_matchers(event, matchers, None);
        }
        Ok(registry)
    }

    /// Load hooks from all 5 sources in order, with deduplication and authority tagging.
    /// Deduplication: by `(event, hook_type, command)` -- later source wins.
    /// Stores project_root and home_dir for later `reload()` and `set_enabled()`.
    pub fn load_all(project_root: &Path, home_dir: &Path) -> Self {
        let mut registry = Self::new();
        registry.project_root = project_root.to_path_buf();
        registry.home_dir = home_dir.to_path_buf();

        // 1. settings.json (backward compat, with .claude fallback)
        let new_settings = project_root.join(".archon/settings.json");
        let old_settings = project_root.join(".claude/settings.json");
        let settings_path = if new_settings.exists() {
            new_settings
        } else if old_settings.exists() {
            tracing::warn!(
                "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                old_settings.display(),
                new_settings.display()
            );
            old_settings
        } else {
            new_settings
        };
        if let Ok(json_str) = std::fs::read_to_string(&settings_path)
            && let Ok(settings) = serde_json::from_str::<SettingsJson>(&json_str)
        {
            for (event, matchers) in settings.hooks {
                registry.register_matchers(event, matchers, Some("project"));
            }
        }

        // 2-5. TOML sources (with .claude fallback for backward compat)
        let sources: [(std::path::PathBuf, std::path::PathBuf, &str); 4] = [
            (
                home_dir.join(".archon/hooks.toml"),
                home_dir.join(".claude/hooks.toml"),
                "user",
            ),
            (
                project_root.join(".archon/hooks.toml"),
                project_root.join(".claude/hooks.toml"),
                "project",
            ),
            (
                project_root.join(".archon/hooks.local.toml"),
                project_root.join(".claude/hooks.local.toml"),
                "local",
            ),
            (
                home_dir.join(".archon/policy/hooks.toml"),
                home_dir.join(".claude/policy/hooks.toml"),
                "policy",
            ),
        ];

        for (new_path, old_path, source_tag) in &sources {
            let effective_path = if new_path.exists() {
                new_path
            } else if old_path.exists() {
                tracing::warn!(
                    "Loading from deprecated path {}. Rename to {} to suppress this warning.",
                    old_path.display(),
                    new_path.display()
                );
                old_path
            } else {
                new_path
            };
            if let Ok(settings) = toml_loader::load_hooks_from_toml(effective_path) {
                for (event, matchers) in settings {
                    registry.register_matchers(event, matchers, Some(source_tag));
                }
            }
        }

        // Deduplicate by (event, hook_type, command) -- keep last
        registry.deduplicate();

        // Load and apply per-id enabled/disabled overrides from hooks.local.toml
        registry.load_overrides();

        registry
    }

    /// Re-load all hook sources from disk and replace internal state.
    /// Preserves the stored `project_root` and `home_dir`.
    pub fn reload(&self) -> Result<(), HookError> {
        let fresh = Self::load_all(&self.project_root, &self.home_dir);

        // Replace entries
        let fresh_entries = fresh
            .entries
            .into_inner()
            .unwrap_or_else(|p| p.into_inner());
        let mut entries = self.entries.write().unwrap_or_else(|p| p.into_inner());
        *entries = fresh_entries;

        // Replace overrides
        let fresh_overrides = fresh
            .enabled_overrides
            .into_inner()
            .unwrap_or_else(|p| p.into_inner());
        let mut overrides = self
            .enabled_overrides
            .write()
            .unwrap_or_else(|p| p.into_inner());
        *overrides = fresh_overrides;

        Ok(())
    }
}
