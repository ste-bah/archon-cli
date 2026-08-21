//! Tell a write branch how much room each declared target file has left.
//!
//! The line cap is enforced when the patch manifest is validated, which is
//! after the agent has done all its work. A branch that grows one file past the
//! cap has its ENTIRE patch rejected — every file in it, however correct.
//!
//! Observed live: a remediation made 21 edits across five files,
//! and lost all of them because one test file would have gone 495 -> 512
//! against a cap of 500. The rejection names the remedy, but by then the work
//! is gone and the branch is a failure.
//!
//! Nothing in the branch input mentioned the cap or the file's current size, so
//! the agent had no way to plan the split. It is measured here and stamped in,
//! so the budget is known before the first edit rather than after the last one.
//!
//! Reads only the declared target paths and a configured number, so it carries
//! no task, PRD or language knowledge.

use std::path::Path;

use serde_json::Value;

use crate::WorkflowV2FanoutItem;

/// Stamp `target_file_budgets` onto every branch that declares target files.
pub(super) fn stamp_target_file_budgets(
    branches: &mut [WorkflowV2FanoutItem],
    target_repository_root: Option<&str>,
    max_source_file_lines: u32,
) {
    let Some(root) = target_repository_root else {
        return;
    };
    for branch in branches {
        // Into `input.item`, beside `required_tools` and `target_files`. The
        // prompt renders that object; a key written at the input's top level is
        // carried in the record and never shown to the agent — which is exactly
        // what happened on the first attempt at this fix.
        let Some(object) = branch.input.get_mut("item").and_then(Value::as_object_mut) else {
            continue;
        };
        let budgets = budgets_for(object.get("target_files"), root, max_source_file_lines);
        if budgets.is_empty() {
            continue;
        }
        object.insert("target_file_budgets".to_string(), Value::Array(budgets));
        object.insert(
            "max_source_file_lines".to_string(),
            Value::from(max_source_file_lines),
        );
    }
}

fn budgets_for(targets: Option<&Value>, root: &str, cap: u32) -> Vec<Value> {
    let Some(paths) = targets.and_then(Value::as_array) else {
        return Vec::new();
    };
    paths
        .iter()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .filter_map(|path| budget_for(path, root, cap))
        .collect()
}

/// A file that does not exist yet has the whole cap available; one that does is
/// reported with what is left. A negative remainder is reported as zero rather
/// than clamped silently — the file is already at or over the cap and any
/// addition to it will reject the patch.
fn budget_for(path: &str, root: &str, cap: u32) -> Option<Value> {
    let absolute = Path::new(root).join(path);
    let lines = match std::fs::read_to_string(&absolute) {
        Ok(text) => u32::try_from(text.lines().count()).unwrap_or(u32::MAX),
        // Unreadable or absent: a new file starts empty, and an unreadable one
        // must not be reported as if its size were known.
        Err(_) => 0,
    };
    Some(serde_json::json!({
        "path": path,
        "current_lines": lines,
        "max_lines": cap,
        "lines_remaining": cap.saturating_sub(lines),
    }))
}

#[cfg(test)]
#[path = "target_budgets_tests.rs"]
mod tests;
