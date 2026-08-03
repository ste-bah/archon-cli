//! Layered config reads for a live workflow run.
//!
//! Split out of `workflow_live.rs` when the SONA tuning wiring pushed that file
//! past the 500-line limit. These five readers were the most self-contained
//! group in it: they share the merge helper, none of them touches the run
//! itself, and every one of them fails the same way — see below.

use std::path::Path;

use archon_core::config::{GeneratedWorkflowConfig, LearningConfig};
use archon_workflow::{WorkflowConfig, WorkflowPolicy};

pub(super) fn live_policy(cwd: &Path, config_path: Option<&Path>) -> WorkflowPolicy {
    WorkflowPolicy {
        require_human_for_dangerous_tools: false,
        ..WorkflowPolicy::from_config(&load_workflow_config(cwd, config_path))
    }
}

/// The merged config layers, with unreadable or unparseable layers skipped.
///
/// Skipping rather than failing throughout: a malformed config layer must not
/// take a workflow run down, and each reader below falls back to its type's
/// defaults, which are the conservative choice in every case.
fn merged_config_layers(cwd: &Path, config_path: Option<&Path>) -> toml::Value {
    use archon_core::config_layers::{deep_merge_toml, discover_config_paths};
    let mut merged = toml::Value::Table(toml::map::Map::new());
    for layer in discover_config_paths(config_path, cwd, None) {
        let Ok(text) = std::fs::read_to_string(&layer.path) else {
            continue;
        };
        let Ok(value) = text.parse::<toml::Value>() else {
            continue;
        };
        merged = deep_merge_toml(merged, value);
    }
    merged
}

fn config_table(merged: &toml::Value, path: &[&str]) -> toml::Value {
    let mut cursor = Some(merged);
    for key in path {
        cursor = cursor.and_then(|value| value.get(key));
    }
    cursor
        .cloned()
        .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()))
}

fn load_workflow_config(cwd: &Path, config_path: Option<&Path>) -> WorkflowConfig {
    config_table(&merged_config_layers(cwd, config_path), &["workflow"])
        .try_into()
        .unwrap_or_else(|_| WorkflowConfig::default())
}

/// Read the layered `[learning]` table.
///
/// Read here rather than threaded from a caller because the planner is the one
/// place that decides a generated run's `learning_hooks`, and those hooks must
/// respect the operator's toggles. The same value now also gates the SONA
/// parameter tuner, so one read answers both consent questions and they cannot
/// disagree.
pub(super) fn load_learning_config(cwd: &Path, config_path: Option<&Path>) -> LearningConfig {
    config_table(&merged_config_layers(cwd, config_path), &["learning"])
        .try_into()
        .unwrap_or_else(|_| LearningConfig::default())
}

pub(super) fn load_generated_workflow_config(
    cwd: &Path,
    config_path: Option<&Path>,
) -> GeneratedWorkflowConfig {
    config_table(
        &merged_config_layers(cwd, config_path),
        &["workflow", "generated"],
    )
    .try_into()
    .unwrap_or_else(|_| GeneratedWorkflowConfig::default())
}
