//! Permission update application logic (REQ-HOOK-016).
//!
//! Hooks returning `updated_permissions` in their result get those updates
//! applied via the [`PermissionStore`] trait. Security: only policy-authority
//! hooks may write to `UserSettings` or `ProjectSettings`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::RwLock;

use super::types::{PermissionUpdate, PermissionUpdateDestination, SourceAuthority};

/// Trait for applying permission updates. Implementations persist changes
/// to the appropriate destination (settings file, in-memory session, etc.).
pub trait PermissionStore: Send + Sync {
    fn add_rules(&self, dest: &PermissionUpdateDestination, rules: &[String])
    -> Result<(), String>;

    fn replace_rules(
        &self,
        dest: &PermissionUpdateDestination,
        rules: &[String],
    ) -> Result<(), String>;

    fn remove_rules(
        &self,
        dest: &PermissionUpdateDestination,
        rules: &[String],
    ) -> Result<(), String>;

    fn set_mode(&self, dest: &PermissionUpdateDestination, mode: &str) -> Result<(), String>;

    fn add_directories(
        &self,
        dest: &PermissionUpdateDestination,
        dirs: &[String],
    ) -> Result<(), String>;

    fn remove_directories(
        &self,
        dest: &PermissionUpdateDestination,
        dirs: &[String],
    ) -> Result<(), String>;
}

/// Apply a batch of permission updates with authority checking.
///
/// Only policy-authority hooks may write to `UserSettings` or `ProjectSettings`.
/// Non-policy hooks attempting this are logged as warnings and silently dropped.
///
/// Returns a list of error messages from any failed store operations.
pub fn apply_permission_updates(
    updates: &[PermissionUpdate],
    source_authority: &SourceAuthority,
    store: &dyn PermissionStore,
) -> Vec<String> {
    let mut errors = Vec::new();

    for update in updates {
        let dest = update.destination();

        // Security: non-policy hooks cannot write to UserSettings/ProjectSettings
        if !matches!(source_authority, SourceAuthority::Policy)
            && matches!(
                dest,
                PermissionUpdateDestination::UserSettings
                    | PermissionUpdateDestination::ProjectSettings
            )
        {
            tracing::warn!(
                dest = ?dest,
                authority = ?source_authority,
                "non-policy hook attempted to modify protected permissions, ignoring"
            );
            continue;
        }

        tracing::info!(update = ?update, "applying permission update");

        let result = match update {
            PermissionUpdate::AddRules { rules, .. } => store.add_rules(dest, rules),
            PermissionUpdate::ReplaceRules { rules, .. } => store.replace_rules(dest, rules),
            PermissionUpdate::RemoveRules { rules, .. } => store.remove_rules(dest, rules),
            PermissionUpdate::SetMode { mode, .. } => store.set_mode(dest, mode),
            PermissionUpdate::AddDirectories { directories, .. } => {
                store.add_directories(dest, directories)
            }
            PermissionUpdate::RemoveDirectories { directories, .. } => {
                store.remove_directories(dest, directories)
            }
        };

        if let Err(e) = result {
            tracing::error!(error = %e, "permission update failed");
            errors.push(e);
        }
    }

    errors
}

// ---------------------------------------------------------------------------
// RuntimePermissionStore — concrete implementation (REQ-HOOK-016)
// ---------------------------------------------------------------------------

/// Runtime permission store that handles in-memory session state
/// and file-based persistent settings (UserSettings / ProjectSettings).
pub struct RuntimePermissionStore {
    /// In-memory session rules: keyed by category (e.g. "rules").
    session_rules: RwLock<HashMap<String, Vec<String>>>,
    /// In-memory session mode override.
    session_mode: RwLock<Option<String>>,
    /// In-memory session directories.
    session_directories: RwLock<Vec<String>>,
    /// Path to user settings: ~/.archon/settings.json
    user_settings_path: PathBuf,
    /// Path to project settings: {project}/.archon/settings.json
    project_settings_path: PathBuf,
}

impl RuntimePermissionStore {
    pub fn new(user_settings: PathBuf, project_settings: PathBuf) -> Self {
        Self {
            session_rules: RwLock::new(HashMap::new()),
            session_mode: RwLock::new(None),
            session_directories: RwLock::new(Vec::new()),
            user_settings_path: user_settings,
            project_settings_path: project_settings,
        }
    }

    /// Resolve a destination to its on-disk path, or `None` for Session.
    fn resolve_path(&self, dest: &PermissionUpdateDestination) -> Option<&PathBuf> {
        match dest {
            PermissionUpdateDestination::UserSettings => Some(&self.user_settings_path),
            PermissionUpdateDestination::ProjectSettings => Some(&self.project_settings_path),
            PermissionUpdateDestination::LocalSettings => Some(&self.project_settings_path),
            PermissionUpdateDestination::Session => None,
        }
    }

    /// Read-modify-write a settings.json file.  The `modify` closure receives
    /// the mutable JSON value.  The file is created (with parent dirs) if absent.
    fn modify_settings_file(
        path: &PathBuf,
        modify: impl FnOnce(&mut serde_json::Value),
    ) -> Result<(), String> {
        let content = std::fs::read_to_string(path).unwrap_or_else(|_| "{}".to_string());
        let mut json: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| format!("parse error: {e}"))?;
        modify(&mut json);
        let output =
            serde_json::to_string_pretty(&json).map_err(|e| format!("serialize error: {e}"))?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::write(path, output).map_err(|e| format!("write error: {e}"))
    }
}

impl PermissionStore for RuntimePermissionStore {
    fn add_rules(
        &self,
        dest: &PermissionUpdateDestination,
        rules: &[String],
    ) -> Result<(), String> {
        if let Some(path) = self.resolve_path(dest) {
            Self::modify_settings_file(path, |json| {
                let perms = json
                    .as_object_mut()
                    .unwrap()
                    .entry("permissions")
                    .or_insert_with(|| serde_json::json!({}));
                let arr = perms
                    .as_object_mut()
                    .unwrap()
                    .entry("rules")
                    .or_insert_with(|| serde_json::json!([]));
                if let Some(a) = arr.as_array_mut() {
                    for rule in rules {
                        a.push(serde_json::Value::String(rule.clone()));
                    }
                }
            })
        } else {
            let mut session = self.session_rules.write().map_err(|e| e.to_string())?;
            let entry = session.entry("rules".to_string()).or_default();
            entry.extend(rules.iter().cloned());
            Ok(())
        }
    }

    fn replace_rules(
        &self,
        dest: &PermissionUpdateDestination,
        rules: &[String],
    ) -> Result<(), String> {
        if let Some(path) = self.resolve_path(dest) {
            Self::modify_settings_file(path, |json| {
                let perms = json
                    .as_object_mut()
                    .unwrap()
                    .entry("permissions")
                    .or_insert_with(|| serde_json::json!({}));
                let arr: Vec<serde_json::Value> = rules
                    .iter()
                    .map(|r| serde_json::Value::String(r.clone()))
                    .collect();
                perms
                    .as_object_mut()
                    .unwrap()
                    .insert("rules".to_string(), serde_json::Value::Array(arr));
            })
        } else {
            let mut session = self.session_rules.write().map_err(|e| e.to_string())?;
            session.insert("rules".to_string(), rules.to_vec());
            Ok(())
        }
    }

    fn remove_rules(
        &self,
        dest: &PermissionUpdateDestination,
        rules: &[String],
    ) -> Result<(), String> {
        if let Some(path) = self.resolve_path(dest) {
            let rules_set: HashSet<&str> = rules.iter().map(|s| s.as_str()).collect();
            Self::modify_settings_file(path, |json| {
                if let Some(perms) = json.get_mut("permissions")
                    && let Some(arr) = perms.get_mut("rules").and_then(|v| v.as_array_mut()) {
                        arr.retain(|v| !v.as_str().map(|s| rules_set.contains(s)).unwrap_or(false));
                    }
            })
        } else {
            let mut session = self.session_rules.write().map_err(|e| e.to_string())?;
            if let Some(existing) = session.get_mut("rules") {
                let rules_set: HashSet<&str> = rules.iter().map(|s| s.as_str()).collect();
                existing.retain(|r| !rules_set.contains(r.as_str()));
            }
            Ok(())
        }
    }

    fn set_mode(&self, dest: &PermissionUpdateDestination, mode: &str) -> Result<(), String> {
        if let Some(path) = self.resolve_path(dest) {
            let mode = mode.to_string();
            Self::modify_settings_file(path, |json| {
                let perms = json
                    .as_object_mut()
                    .unwrap()
                    .entry("permissions")
                    .or_insert_with(|| serde_json::json!({}));
                perms
                    .as_object_mut()
                    .unwrap()
                    .insert("mode".to_string(), serde_json::Value::String(mode));
            })
        } else {
            let mut session_mode = self.session_mode.write().map_err(|e| e.to_string())?;
            *session_mode = Some(mode.to_string());
            Ok(())
        }
    }

    fn add_directories(
        &self,
        dest: &PermissionUpdateDestination,
        dirs: &[String],
    ) -> Result<(), String> {
        if let Some(path) = self.resolve_path(dest) {
            Self::modify_settings_file(path, |json| {
                let perms = json
                    .as_object_mut()
                    .unwrap()
                    .entry("permissions")
                    .or_insert_with(|| serde_json::json!({}));
                let arr = perms
                    .as_object_mut()
                    .unwrap()
                    .entry("directories")
                    .or_insert_with(|| serde_json::json!([]));
                if let Some(a) = arr.as_array_mut() {
                    for d in dirs {
                        a.push(serde_json::Value::String(d.clone()));
                    }
                }
            })
        } else {
            let mut session_dirs = self
                .session_directories
                .write()
                .map_err(|e| e.to_string())?;
            session_dirs.extend(dirs.iter().cloned());
            Ok(())
        }
    }

    fn remove_directories(
        &self,
        dest: &PermissionUpdateDestination,
        dirs: &[String],
    ) -> Result<(), String> {
        if let Some(path) = self.resolve_path(dest) {
            let dirs_set: HashSet<&str> = dirs.iter().map(|s| s.as_str()).collect();
            Self::modify_settings_file(path, |json| {
                if let Some(perms) = json.get_mut("permissions")
                    && let Some(arr) = perms.get_mut("directories").and_then(|v| v.as_array_mut()) {
                        arr.retain(|v| !v.as_str().map(|s| dirs_set.contains(s)).unwrap_or(false));
                    }
            })
        } else {
            let mut session_dirs = self
                .session_directories
                .write()
                .map_err(|e| e.to_string())?;
            let dirs_set: HashSet<&str> = dirs.iter().map(|s| s.as_str()).collect();
            session_dirs.retain(|d| !dirs_set.contains(d.as_str()));
            Ok(())
        }
    }
}
