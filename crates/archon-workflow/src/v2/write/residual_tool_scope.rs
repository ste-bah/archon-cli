//! The host's scope of a residual round's required tools (Issue-121
//! follow-up).
//!
//! A write branch is stamped with every tool its canonical tasks declare
//! (`stamp_required_tools_from_universe`), and the adapter demands an
//! invocation of each before an accepted or no-op result stands. A
//! host-planned residual round (its item names the files the host granted
//! it, `residual_expansion_paths`) is one bounded fix of named gaps over
//! possibly several tasks; demanding every tool of every task made a no-op
//! round -- the gap already gone -- fail for tools its gaps and files never
//! concern.
//!
//! For such a branch the host also stamps, per declared tool, what its tasks'
//! OWN metadata ties it to: the task's focused tests, acceptance criteria and
//! artifact requirements that name the tool (as a token) name files -- a
//! declared path of the task, by path or file name -- or file kinds (a
//! `.ext` token). A tool no such line ties to anything carries the files its
//! requiring tasks declare instead. The adapter then asks for exactly the
//! tools the round needs (`scoped_required_tools`). Derived from the task
//! universe alone, by the branch's own canonical task ids; any value already
//! on an item is removed first, so no authored item can narrow its own tools.

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Value, json};

use crate::task_universe::{WorkflowV2TaskUniverse, WorkflowV2TaskUniverseTask};
use crate::tool_declarations::{REQUIRED_TOOL_SCOPE_KEY, raw_tool_name, text_names_tool};
use crate::v2::script::residual_plan::RESIDUAL_ITEM_PATHS_KEY;
use crate::v2::verification::path_ownership::{
    DeclaredPathForm, declared_path_form, declared_paths_of,
};

pub(super) fn stamp_residual_tool_scope(
    branches: &mut [crate::WorkflowV2FanoutItem],
    task_universe: Option<&WorkflowV2TaskUniverse>,
    repository_root: Option<&str>,
) {
    for branch in branches {
        let Some(item) = branch.input.get_mut("item").and_then(Value::as_object_mut) else {
            continue;
        };
        item.remove(REQUIRED_TOOL_SCOPE_KEY);
        let (Some(universe), Some(root)) = (task_universe, repository_root) else {
            continue;
        };
        if !item.contains_key(RESIDUAL_ITEM_PATHS_KEY) {
            continue;
        }
        let claimed: Vec<&str> = item
            .get("canonical_task_ids")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let tasks: Vec<&WorkflowV2TaskUniverseTask> = universe
            .tasks
            .iter()
            .filter(|task| claimed.contains(&task.canonical_task_id.as_str()))
            .collect();
        let tools: Vec<&str> = item
            .get("required_tools")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let scope: Vec<Value> = tools
            .iter()
            .map(|tool| tool_scope(tool, &tasks, Path::new(root)))
            .collect();
        if !scope.is_empty() {
            item.insert(REQUIRED_TOOL_SCOPE_KEY.to_string(), Value::Array(scope));
        }
    }
}

/// What `tasks`' metadata ties the declared `tool` to.
fn tool_scope(tool: &str, tasks: &[&WorkflowV2TaskUniverseTask], root: &Path) -> Value {
    let key = raw_tool_name(tool).to_ascii_lowercase();
    let mut files = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    let mut task_files = BTreeSet::new();
    for task in tasks
        .iter()
        .filter(|task| task.required_tools.iter().any(|t| t == tool))
    {
        let declared: Vec<String> = declared_paths_of(task)
            .into_iter()
            .filter_map(|entry| match declared_path_form(&entry, root) {
                DeclaredPathForm::Repo(path) => Some(path),
                _ => None,
            })
            .collect();
        task_files.extend(declared.iter().cloned());
        for line in task
            .focused_tests
            .iter()
            .chain(&task.acceptance_criteria)
            .chain(&task.artifact_requirements)
            .filter(|line| text_names_tool(line, &key))
        {
            for path in &declared {
                let name = path.rsplit('/').next().unwrap_or(path);
                if line.contains(path.as_str()) || names_word(line, name) {
                    files.insert(path.clone());
                }
            }
            kinds.extend(file_kinds(line));
        }
    }
    json!({"tool": tool, "files": files, "extensions": kinds, "task_files": task_files})
}

/// Whether `line` holds `word` delimited by neither a path nor a word
/// character on either side.
fn names_word(line: &str, word: &str) -> bool {
    let part = |c: char| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/');
    !word.is_empty()
        && line.match_indices(word).any(|(at, _)| {
            !line[..at].chars().next_back().is_some_and(part)
                && !line[at + word.len()..].chars().next().is_some_and(part)
        })
}

/// The `.ext` tokens of `line` (`.cfg`, `*.sql`): a kind of file it names.
fn file_kinds(line: &str) -> Vec<String> {
    line.split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | ',' | '(' | ')' | ';'))
        .map(|word| {
            word.trim_start_matches('*')
                .trim_end_matches([':', '.', ','])
        })
        .filter(|word| {
            word.len() > 1
                && word.starts_with('.')
                && word[1..].chars().all(|c| c.is_ascii_alphanumeric())
                && word[1..].chars().any(|c| c.is_ascii_alphabetic())
        })
        .map(str::to_string)
        .collect()
}

/// The declared, policed tools a residual round's CHANGED-but-unreported
/// paths need that the adapter never demanded (by the host's scope stamp on
/// `input`), sorted.
///
/// The adapter scopes a round's tools by its claim, the files its envelope
/// says it changed and the files it was granted, and refuses an accepted
/// result that did not exercise every tool so owed. A path the worktree
/// shows changed that the envelope never named was never weighed there: a
/// tool it is tied to (by file or kind, or -- for a tool tied to nothing --
/// a file its task declares) that none of the weighed ones owed has no
/// proof demanded, and the branch is refused ([`unreported_tool_rejection`]).
/// `unreported` excludes paths the landing drops anyway.
pub(super) fn owed_by_unreported(
    input: &Value,
    reported: &[String],
    unreported: &[String],
) -> Vec<String> {
    let item = input.get("item").unwrap_or(&Value::Null);
    let Some(scope) = item.get(REQUIRED_TOOL_SCOPE_KEY).and_then(Value::as_array) else {
        return Vec::new();
    };
    let strings = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|path| path.trim().trim_start_matches("./").to_string())
            .collect()
    };
    let covers = |declared: &str, path: &str| {
        path == declared || path.starts_with(&format!("{}/", declared.trim_end_matches('/')))
    };
    let claim = item.get("task").and_then(Value::as_str).unwrap_or_default();
    let mut weighed: Vec<String> = reported
        .iter()
        .map(|path| path.trim().trim_start_matches("./").to_string())
        .collect();
    weighed.extend(strings(&item[RESIDUAL_ITEM_PATHS_KEY]));
    let mut owed = BTreeSet::new();
    for entry in scope {
        let Some(tool) = entry["tool"].as_str() else {
            continue;
        };
        let key = raw_tool_name(tool).to_ascii_lowercase();
        if !crate::v2::agent_adapter::is_policed_tool(tool) || text_names_tool(claim, &key) {
            continue;
        }
        let files = strings(&entry["files"]);
        let kinds = strings(&entry["extensions"]);
        let untied = files.is_empty() && kinds.is_empty();
        let declared = strings(&entry["task_files"]);
        let tied = |path: &String| {
            files.iter().any(|file| covers(file, path))
                || kinds.iter().any(|kind| path.ends_with(kind.as_str()))
        };
        // What the adapter already owed (and so proved, on an accepted
        // result): a tied file it weighed, or -- untied -- a reported task file.
        let demanded = if untied {
            reported
                .iter()
                .any(|path| declared.iter().any(|file| covers(file, path)))
        } else {
            weighed.iter().any(tied)
        };
        let needed = if untied {
            unreported
                .iter()
                .any(|path| declared.iter().any(|file| covers(file, path)))
        } else {
            unreported.iter().any(tied)
        };
        if needed && !demanded {
            owed.insert(tool.to_string());
        }
    }
    owed.into_iter().collect()
}

/// The result a branch is refused with when [`owed_by_unreported`] names a
/// tool: nothing lands.
pub(super) fn unreported_tool_rejection(
    item_id: &str,
    canonical_task_ids: &[String],
    unreported: &[String],
    tools: &[String],
) -> crate::v2::WorkflowV2Result {
    use super::errors::{
        branch_validation_failure_fields, sanitize_v2_path_segment, truncate_for_result,
    };
    use crate::v2::{
        BranchFailureKind, WorkflowV2Evidence, WorkflowV2ResidualGap, WorkflowV2Result,
    };
    let failure_kind = BranchFailureKind::Contract;
    let (status, evidence_kind, severity) = branch_validation_failure_fields(&failure_kind);
    let summary = format!(
        "write item '{item_id}' changed {} path(s) its envelope did not report ({}), which need          the declared tool(s) {}; the patch was not captured",
        unreported.len(),
        unreported.join(", "),
        tools.join(", ")
    );
    let mut result = WorkflowV2Result {
        status,
        summary: truncate_for_result(&summary, 2_000),
        ..WorkflowV2Result::default()
    };
    result.evidence.push(WorkflowV2Evidence::new(
        evidence_kind,
        "a residual round changed files it did not report that need declared tools; the          rejection was retained as typed remediation data",
    ));
    result.residual_gaps.push(WorkflowV2ResidualGap {
        id: format!(
            "required_tool_unreported_change_{}",
            sanitize_v2_path_segment(item_id)
        ),
        description: truncate_for_result(
            &format!(
                "{summary}. Report every file you change in files_changed, and exercise each                  declared tool a changed file needs."
            ),
            1_000,
        ),
        severity: Some(severity.to_string()),
    });
    result.data = json!({
        "branch_id": item_id,
        "item_id": item_id,
        "canonical_task_ids": canonical_task_ids,
        "branch_error_from_runtime": true,
        "failure_kind": failure_kind,
        "error": truncate_for_result(&summary, 2_000),
        "patch_landed": false,
    });
    result
}

#[cfg(test)]
#[path = "residual_tool_scope_tests.rs"]
mod tests;
