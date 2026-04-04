//! Configuration diffing utilities for hot-reload support.
//!
//! Compares two [`ArchonConfig`] instances and returns a list of dotted key
//! paths that differ. Also determines which keys are safe to hot-reload
//! versus those requiring a restart.

use toml::Value;

use crate::config::ArchonConfig;

// ---------------------------------------------------------------------------
// diff_configs
// ---------------------------------------------------------------------------

/// Compare two [`ArchonConfig`] values and return a list of dotted key paths
/// that differ between them.
///
/// Both configs are serialized to TOML [`Value`] trees and walked
/// recursively. Leaf values that differ produce entries like
/// `"permissions.mode"` or `"api.default_model"`.
pub fn diff_configs(old: &ArchonConfig, new: &ArchonConfig) -> Vec<String> {
    let old_val = match toml::Value::try_from(old) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let new_val = match toml::Value::try_from(new) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };

    let mut diffs = Vec::new();
    walk_diff(&old_val, &new_val, "", &mut diffs);
    diffs
}

// ---------------------------------------------------------------------------
// Reloadability classification
// ---------------------------------------------------------------------------

/// Determine if a dotted config key is safe to hot-reload without restart.
///
/// Reloadable prefixes: `permissions.*`, `hooks.*`, `personality.*`,
/// `cost.*`, `mcp.*`, `tools.*`, `context.*`, `consciousness.*`,
/// `tui.*`, `session.*`, `checkpoint.*`, `memory.*`.
///
/// NOT reloadable (require restart): `api.*`, `identity.*`, `logging.*`.
pub fn is_reloadable(key: &str) -> bool {
    !key.starts_with("api.")
        && !key.starts_with("identity.")
        && !key.starts_with("logging.")
}

/// Filter a list of changed keys to only those that are NOT hot-reloadable
/// (i.e., require a restart to take effect).
pub fn non_reloadable_changes(changes: &[String]) -> Vec<String> {
    changes
        .iter()
        .filter(|k| !is_reloadable(k))
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Recursively walk two TOML value trees and collect dotted key paths where
/// the leaf values differ.
fn walk_diff(old: &Value, new: &Value, prefix: &str, diffs: &mut Vec<String>) {
    match (old, new) {
        (Value::Table(old_map), Value::Table(new_map)) => {
            // Keys present in both — recurse
            for (key, old_val) in old_map {
                let full_key = dotted_key(prefix, key);
                match new_map.get(key) {
                    Some(new_val) => walk_diff(old_val, new_val, &full_key, diffs),
                    None => collect_leaves(old_val, &full_key, diffs),
                }
            }
            // Keys only in new
            for (key, new_val) in new_map {
                if !old_map.contains_key(key) {
                    let full_key = dotted_key(prefix, key);
                    collect_leaves(new_val, &full_key, diffs);
                }
            }
        }
        // Both are non-table — compare directly
        (a, b) => {
            if a != b {
                diffs.push(prefix.to_string());
            }
        }
    }
}

/// Collect all leaf key paths under a TOML value (used when a whole subtree
/// was added or removed).
fn collect_leaves(value: &Value, prefix: &str, diffs: &mut Vec<String>) {
    match value {
        Value::Table(map) => {
            for (key, val) in map {
                let full_key = dotted_key(prefix, key);
                collect_leaves(val, &full_key, diffs);
            }
        }
        _ => {
            diffs.push(prefix.to_string());
        }
    }
}

/// Build a dotted key path, handling the empty-prefix case.
fn dotted_key(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_reloadable_cost() {
        assert!(is_reloadable("cost.warn_threshold"));
    }

    #[test]
    fn is_not_reloadable_logging() {
        assert!(!is_reloadable("logging.level"));
    }

    #[test]
    fn dotted_key_empty_prefix() {
        assert_eq!(dotted_key("", "foo"), "foo");
    }

    #[test]
    fn dotted_key_with_prefix() {
        assert_eq!(dotted_key("a.b", "c"), "a.b.c");
    }
}
