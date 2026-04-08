//! TOML-based hook configuration loader.
//!
//! Supports loading hooks from `.toml` files with the following structure:
//!
//! ```toml
//! [hooks.PreToolUse]
//! matchers = [
//!   { matcher = "Bash", hooks = [
//!     { type = "command", command = "scripts/check.sh", timeout = 10 }
//!   ]}
//! ]
//! ```

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use super::types::{HookError, HookEvent, HookMatcher, HooksSettings};

// ---------------------------------------------------------------------------
// Intermediate TOML deserialization types
// ---------------------------------------------------------------------------

/// Per-event wrapper: `[hooks.EventName]` is a table containing `matchers = [...]`.
#[derive(Deserialize)]
struct TomlEventEntry {
    #[serde(default)]
    matchers: Vec<HookMatcher>,
}

/// Top-level TOML file structure: `[hooks]` table keyed by event name.
#[derive(Deserialize)]
struct TomlHooksFile {
    #[serde(default)]
    hooks: HashMap<HookEvent, TomlEventEntry>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a TOML string into `HooksSettings`.
pub fn parse_hooks_toml(content: &str) -> Result<HooksSettings, toml::de::Error> {
    let file: TomlHooksFile = toml::from_str(content)?;
    let settings: HooksSettings = file
        .hooks
        .into_iter()
        .map(|(event, entry)| (event, entry.matchers))
        .collect();
    Ok(settings)
}

/// Load hooks from a TOML file path.
///
/// Returns `Ok(empty)` on missing file (silently skipped).
/// Returns `Err` on I/O errors (other than not-found) or parse errors.
pub fn load_hooks_from_toml(path: &Path) -> Result<HooksSettings, HookError> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HashMap::new());
        }
        Err(e) => {
            return Err(HookError::ConfigError(format!(
                "failed to read {}: {e}",
                path.display()
            )));
        }
    };
    parse_hooks_toml(&content)
        .map_err(|e| HookError::ConfigError(format!("TOML parse error in {}: {e}", path.display())))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::types::{HookCommandType, HookEvent};

    #[test]
    fn parse_empty_toml() {
        let settings = parse_hooks_toml("").unwrap();
        assert!(settings.is_empty());
    }

    #[test]
    fn parse_single_event_single_hook() {
        let toml_str = r#"
[hooks.PreToolUse]
matchers = [
  { matcher = "Bash", hooks = [
    { type = "command", command = "scripts/check.sh", timeout = 10 }
  ]}
]
"#;
        let settings = parse_hooks_toml(toml_str).unwrap();
        assert!(settings.contains_key(&HookEvent::PreToolUse));
        let matchers = &settings[&HookEvent::PreToolUse];
        assert_eq!(matchers.len(), 1);
        assert_eq!(matchers[0].matcher.as_deref(), Some("Bash"));
        assert_eq!(matchers[0].hooks.len(), 1);
        assert_eq!(matchers[0].hooks[0].hook_type, HookCommandType::Command);
        assert_eq!(matchers[0].hooks[0].command, "scripts/check.sh");
        assert_eq!(matchers[0].hooks[0].timeout, Some(10));
    }

    #[test]
    fn parse_multiple_events() {
        let toml_str = r#"
[hooks.PreToolUse]
matchers = [
  { hooks = [{ type = "command", command = "pre.sh" }] }
]

[hooks.PostToolUse]
matchers = [
  { hooks = [{ type = "command", command = "post.sh" }] }
]
"#;
        let settings = parse_hooks_toml(toml_str).unwrap();
        assert_eq!(settings.len(), 2);
        assert!(settings.contains_key(&HookEvent::PreToolUse));
        assert!(settings.contains_key(&HookEvent::PostToolUse));
    }

    #[test]
    fn parse_invalid_toml_returns_error() {
        let result = parse_hooks_toml("not valid [[[toml");
        assert!(result.is_err());
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let path = Path::new("/nonexistent/hooks.toml");
        let settings = load_hooks_from_toml(path).unwrap();
        assert!(settings.is_empty());
    }

    #[test]
    fn load_real_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks.toml");
        std::fs::write(
            &path,
            r#"
[hooks.SessionStart]
matchers = [
  { hooks = [{ type = "command", command = "echo hello" }] }
]
"#,
        )
        .unwrap();

        let settings = load_hooks_from_toml(&path).unwrap();
        assert!(settings.contains_key(&HookEvent::SessionStart));
    }

    #[test]
    fn load_malformed_toml_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "{{{{not toml").unwrap();

        let result = load_hooks_from_toml(&path);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("TOML parse error"));
    }
}
