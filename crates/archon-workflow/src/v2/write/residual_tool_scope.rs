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

#[cfg(test)]
#[path = "residual_tool_scope_tests.rs"]
mod tests;
