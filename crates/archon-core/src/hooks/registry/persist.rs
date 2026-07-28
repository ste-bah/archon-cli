use std::collections::HashMap;

use super::HookRegistry;
use crate::hooks::types::HookError;

impl HookRegistry {
    /// Enable or disable a hook by id, persisting to
    /// `<project_root>/.archon/hooks.local.toml`.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<(), HookError> {
        // Update in-memory override.
        {
            let mut overrides = self
                .enabled_overrides
                .write()
                .unwrap_or_else(|p| p.into_inner());
            overrides.insert(id.to_string(), enabled);
        }

        // Persist to disk.
        self.write_overrides_file(id, enabled)
    }

    /// Read `[overrides]` from `<project_root>/.archon/hooks.local.toml` and
    /// merge into the in-memory `enabled_overrides` map.
    pub(super) fn load_overrides(&self) {
        let path = self.project_root.join(".archon/hooks.local.toml");
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return,
        };

        // Parse the TOML looking for [overrides] section.
        let mut in_overrides = false;
        let mut overrides = HashMap::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "[overrides]" {
                in_overrides = true;
                continue;
            }
            if in_overrides {
                if trimmed.starts_with('[') {
                    // Next section, stop
                    break;
                }
                if let Some((key, value)) = trimmed.split_once('=') {
                    let key = key.trim().trim_matches('"');
                    let value = value.trim().trim_matches('"');
                    if let Ok(b) = value.parse::<bool>() {
                        overrides.insert(key.to_string(), b);
                    }
                }
            }
        }

        if !overrides.is_empty() {
            let mut map = self
                .enabled_overrides
                .write()
                .unwrap_or_else(|p| p.into_inner());
            for (k, v) in overrides {
                map.entry(k).or_insert(v);
            }
        }
    }

    /// Write the full `enabled_overrides` map to
    /// `<project_root>/.archon/hooks.local.toml`, preserving non-`[overrides]`
    /// sections.
    fn write_overrides_file(&self, _id: &str, _enabled: bool) -> Result<(), HookError> {
        let path = self.project_root.join(".archon/hooks.local.toml");

        // Ensure parent directory exists.
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                HookError::IoError(std::io::Error::other(format!("create .archon dir: {e}")))
            })?;
        }

        let overrides = self
            .enabled_overrides
            .read()
            .unwrap_or_else(|p| p.into_inner());

        // Read existing file content to preserve non-[overrides] sections.
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        let new_content = merge_overrides_into_toml(&existing, &overrides);

        std::fs::write(&path, &new_content).map_err(|e| {
            HookError::IoError(std::io::Error::other(format!(
                "write hooks.local.toml: {e}"
            )))
        })?;

        Ok(())
    }
}

/// Merge the current `[overrides]` map into an existing hooks.local.toml
/// file, preserving non-override sections.
fn merge_overrides_into_toml(existing: &str, overrides: &HashMap<String, bool>) -> String {
    let mut out = String::new();
    let mut in_overrides = false;

    // Preserve all lines except the old [overrides] section.
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed == "[overrides]" {
            in_overrides = true;
            continue;
        }
        if in_overrides && trimmed.starts_with('[') {
            in_overrides = false;
            // Fall through to emit this line
        }
        if !in_overrides {
            out.push_str(line);
            out.push('\n');
        }
    }

    // Ensure trailing newline before appending overrides.
    if !out.ends_with('\n') {
        out.push('\n');
    }

    // Append fresh overrides section.
    if !overrides.is_empty() {
        out.push_str("[overrides]\n");
        let mut keys: Vec<&String> = overrides.keys().collect();
        keys.sort();
        for k in keys {
            let v = overrides[k];
            out.push_str(&format!("{k} = {v}\n"));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_enabled_persists_and_toggles() {
        use tempfile::TempDir;

        let project_dir = TempDir::new().unwrap();
        let home_dir = TempDir::new().unwrap();
        let archon_dir = project_dir.path().join(".archon");
        std::fs::create_dir_all(&archon_dir).unwrap();

        // Write a fixture hooks.toml.
        let fixture = r#"
[hooks.PreToolUse]
matchers = [
  { matcher = "Bash", hooks = [
    { type = "command", command = "guard-secrets" }
  ]}
]
"#;
        std::fs::write(archon_dir.join("hooks.toml"), fixture).unwrap();

        let reg = HookRegistry::load_all(project_dir.path(), home_dir.path());
        let summaries = reg.summaries();
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].enabled, "hook must default to enabled");
        let hook_id = summaries[0].id.clone();

        // Disable the hook.
        reg.set_enabled(&hook_id, false).unwrap();

        // Verify hooks.local.toml was created.
        let local_path = archon_dir.join("hooks.local.toml");
        assert!(local_path.exists(), "hooks.local.toml must be created");
        let content = std::fs::read_to_string(&local_path).unwrap();
        assert!(
            content.contains("[overrides]"),
            "must contain [overrides] section"
        );
        assert!(content.contains(&hook_id), "must contain the hook id");

        // Reload and verify the hook is now disabled.
        let reg2 = HookRegistry::load_all(project_dir.path(), home_dir.path());
        let summaries2 = reg2.summaries();
        assert_eq!(summaries2.len(), 1);
        assert!(
            !summaries2[0].enabled,
            "hook must show as disabled after reload"
        );
        assert_eq!(
            summaries2[0].id, hook_id,
            "id must be stable across reloads"
        );
    }

    #[test]
    fn merge_overrides_preserves_non_overrides_sections() {
        let existing = "[other]\nfoo = \"bar\"\n\n[overrides]\nold = true\n";
        let mut overrides = HashMap::new();
        overrides.insert("h12345678".to_string(), false);
        let merged = merge_overrides_into_toml(existing, &overrides);
        assert!(merged.contains("[other]"));
        assert!(merged.contains("foo = \"bar\""));
        assert!(
            !merged.contains("old = true"),
            "old override must be removed"
        );
        assert!(merged.contains("h12345678 = false"));
    }
}
