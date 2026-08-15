//! Telling a write branch where the repository actually is.
//!
//! Verification items carry `target_repository_root` and their prompt states
//! that repository source resolves against it. Implementation and remediation
//! items mostly did not: across one run's 8,409 agent transcripts the value
//! reached 40/40 verification items but only 14/40 implementation and 4/40
//! remediation items.
//!
//! An agent without it sees `project_artifact_root` and its own worktree path,
//! and guesses. The guess is consistently the artifact root, so it looks for
//! `<project>/crates/...` — a path that cannot exist, because `crates/` lives
//! in the target repository. That produced 234 `File does not exist` misses on
//! files that were present the whole time, and the agent spends turns hunting
//! for them instead of doing the work.

use serde_json::Value;

use crate::v2::WorkflowV2FanoutItem;

/// Stamp the repository root into each branch's `input.item`.
///
/// Written beside `required_tools` and `target_files` rather than at the
/// input's top level: the prompt renders the item object, and a key placed
/// outside it is carried in the record but never shown to the agent.
///
/// An existing value is left alone. Only the host knows this path, but a
/// branch that already carries one has been given it deliberately and
/// overwriting it would silently change where that agent resolves paths.
pub(crate) fn stamp_target_repository_root(
    branches: &mut [WorkflowV2FanoutItem],
    target_repository_root: Option<&str>,
) {
    let Some(root) = target_repository_root
        .map(str::trim)
        .filter(|r| !r.is_empty())
    else {
        return;
    };
    for branch in branches {
        let Some(object) = branch.input.get_mut("item").and_then(Value::as_object_mut) else {
            continue;
        };
        if object
            .get("target_repository_root")
            .and_then(Value::as_str)
            .is_some_and(|existing| !existing.trim().is_empty())
        {
            continue;
        }
        object.insert(
            "target_repository_root".to_string(),
            Value::String(root.to_string()),
        );
    }
}

#[cfg(test)]
#[path = "repository_root_tests.rs"]
mod tests;
