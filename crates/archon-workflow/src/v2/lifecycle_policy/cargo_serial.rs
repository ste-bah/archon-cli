//! Scheduling protection for commands that may share mutable build resources.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::generated_lifecycle_support::raw_strings;
use crate::v2::WorkflowV2FanoutItem;

/// Shared resource lane. The exported name is retained for caller compatibility.
///
/// Only a scheduling identity: the agent's behavioural role stays in the
/// branch call's options, which this retagging never touches.
pub const CARGO_SERIAL_ROLE: &str = "build-serial";

/// Inspect declared commands, never incidental mentions in evidence or prompts.
/// Unknown programs and free-form agent tasks conservatively share the lane.
pub fn item_has_cargo_commands(item: &Value) -> bool {
    declared_lane(item)
        .unwrap_or_else(|| item.get("task").is_some() || item.get("work_type").is_some())
}

fn declared_lane(item: &Value) -> Option<bool> {
    let commands = raw_strings(item, &["focused_verification", "commands", "command"]);
    (!commands.is_empty()).then(|| commands.iter().any(|text| runs_resource_command(text)))
}

fn runs_resource_command(text: &str) -> bool {
    crate::command_resources::needs_serial_lane(text)
}

/// Retag resource-using fanout items into [`CARGO_SERIAL_ROLE`].
///
/// Checks both the branch input's `item` object (where fanout wraps the plan
/// item) and the input root (where bare items land), so the tag survives
/// either layout. The input JSON itself is deliberately untouched — item input
/// hashes drive branch-outcome reuse across resumes, and a stamp inside the
/// input would invalidate every cached verification outcome once.
pub fn tag_cargo_serial_roles(items: Vec<WorkflowV2FanoutItem>) -> Vec<WorkflowV2FanoutItem> {
    items
        .into_iter()
        .map(|mut item| {
            let selected = item.input.get("item").unwrap_or(&item.input);
            if item_has_cargo_commands(selected) || declared_lane(&item.input) == Some(true) {
                item.role = CARGO_SERIAL_ROLE.to_string();
            }
            item
        })
        .collect()
}

/// Resource-using branches run serially; inspections keep the configured width.
pub fn cargo_serial_role_limits() -> BTreeMap<String, usize> {
    BTreeMap::from([(CARGO_SERIAL_ROLE.to_string(), 1)])
}

#[cfg(test)]
#[path = "cargo_serial_tests.rs"]
mod tests;
