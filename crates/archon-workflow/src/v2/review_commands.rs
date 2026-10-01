//! REM-16: a review map branch may RUN commands, read-only, and every
//! finding it returns says what it ran.
//!
//! The critic map stages ran with no shell, so a reviewer could only read:
//! "the focused tests pass" was attested from the task's own report, never
//! executed (a finding can say as much: "only statically attested"). A review map
//! branch now gets the shell a per-task verifier gets. It stays READ-ONLY:
//! every read-only call runs under the host's OS write boundary (Batch G,
//! `workflow_live_v2_call_boundary::read_only_boundary`), which seals the
//! canonical checkout, the project and the run store, so a reviewer's shell
//! can write only build-output directories and temporary space. A change a
//! reviewer wants to try (a mutation that shows a test guards nothing) is
//! made on a temporary COPY of the repository, never on the real tree.
//!
//! Three host pieces, all keyed on the review contract, never on a call id:
//!
//! - [`grant_review_commands`] stamps the rule on each branch input the host
//!   is about to dispatch, AFTER the reuse split, and the grant only when the
//!   host can bound a shell (the platform's OS boundary is available and a
//!   boundary scope is drawn); otherwise the branch is told it has none. It
//!   returns each branch's reuse identity (the item as authored): the
//!   recorded outcome is filed under it ([`restore_review_identity`]), so a
//!   resume matches it whether or not the host that recorded it granted a
//!   shell. A granted branch runs under the host's repository-tree tripwire
//!   (`workflow_live_v2_read_only_tree`): any change fails it and is reverted;
//! - [`mark_review_command_access`] marks which shell a branch had, so its
//!   record says so;
//! - the map attachment stamps each finding of a marked branch with the
//!   commands that branch ran ([`stamp_branch_commands`]); an unmarked
//!   branch (recorded before the grant) is left exactly as recorded, so no
//!   host finding id of an earlier run moves.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::v2::WorkflowV2Result;
use crate::v2::scheduler::{WorkflowV2BranchOutcome, WorkflowV2FanoutItem};

/// Branch-input key the host's review execution rule travels under.
pub const REVIEW_EXECUTION_INPUT_KEY: &str = "review_execution";

/// Result-data key marking a branch that ran with the host's shell grant.
pub const REVIEW_COMMAND_ACCESS_KEY: &str = "review_command_access";

/// [`REVIEW_COMMAND_ACCESS_KEY`] for a branch that had the shell.
pub const REVIEW_READ_ONLY_SHELL: &str = "read_only_shell";

/// [`REVIEW_COMMAND_ACCESS_KEY`] for a branch the host could not bound, so
/// it ran with no shell (Major 2: never a shell without the OS boundary).
pub const REVIEW_NO_SHELL: &str = "no_shell";

/// `stage_extra` key marking the host's read-only shell grant: the stage
/// gets read-only tools plus Bash, never full access (`stage_command_policy`).
pub const READ_ONLY_SHELL_GRANT_KEY: &str = "read_only_shell";

/// REM-10: the review kinds of a late review (the prelude's
/// `reviewMovedTasks`), whose maps are `*-moved-N-map` at stage `map`.
pub const ADVERSARIAL_MOVED_KIND: &str = "adversarial_findings_moved";
/// See [`ADVERSARIAL_MOVED_KIND`].
pub const COVERAGE_MOVED_KIND: &str = "uncovered_requirements_moved";

/// Finding key the host stamps with the commands its branch ran.
pub const REVIEW_COMMANDS_RUN_KEY: &str = "review_commands_run";

/// What a granted reviewer is told, beside its input.
pub const REVIEW_EXECUTION_RULE: &str = "The host granted THIS review a READ-ONLY shell (Bash). Use it to falsify the task: run its declared focused tests and any build, test or inspection command whose output can prove or disprove its claims, from repository_root. Your shell cannot write the repository, the project or the run store (the host refuses those writes at the operating-system level, and any change to the repository tree fails this review and is reverted; only build-output and temporary directories are writable). To try a change -- for example to show that a test does not catch the defect it claims to guard -- copy the repository to a temporary directory and change and run the COPY, never the real tree. Record every command you ran, with its exit code, in commands_run. In every finding put under `commands` the exact commands whose output establishes it, or an empty list when it rests on reading alone; a claim that a test passes or fails must name the command that ran it. The host also stamps every finding with the commands this branch ran.";

/// What a reviewer the host could not bound is told.
pub const REVIEW_NO_SHELL_RULE: &str = "The host could NOT bound a shell on this platform, so this review has NO shell: judge the task by reading its code, tests and artifacts. Never claim a command ran or a test passed: in every finding put `commands: []`, and say where a claim needs a command no one ran.";

/// Stamp the review rule on each branch the host is about to dispatch, and
/// with `shell` the read-only shell grant; returns each branch's reuse
/// identity (`reuse_identity`, the item as authored) by branch id.
pub fn grant_review_commands(
    items: &mut [WorkflowV2FanoutItem],
    shell: bool,
) -> BTreeMap<String, String> {
    let mut identities = BTreeMap::new();
    for item in items.iter_mut() {
        identities.insert(
            item.id.clone(),
            crate::v2::reuse_identity::reuse_identity(item),
        );
        let Some(object) = item.input.as_object_mut() else {
            continue;
        };
        let rule = if shell {
            REVIEW_EXECUTION_RULE
        } else {
            REVIEW_NO_SHELL_RULE
        };
        object.insert(
            REVIEW_EXECUTION_INPUT_KEY.to_string(),
            Value::String(rule.to_string()),
        );
        if !shell {
            continue;
        }
        let extra = object
            .entry("stage_extra".to_string())
            .or_insert_with(|| json!({}));
        if !extra.is_object() {
            *extra = json!({});
        }
        if let Some(extra) = extra.as_object_mut() {
            extra.insert(READ_ONLY_SHELL_GRANT_KEY.to_string(), Value::Bool(true));
            let tools = extra
                .entry("allowed_tools".to_string())
                .or_insert_with(|| json!([]));
            if !tools.is_array() {
                *tools = json!([]);
            }
            if let Some(tools) = tools.as_array_mut()
                && !tools.iter().any(|tool| tool.as_str() == Some("Bash"))
            {
                tools.push(json!("Bash"));
            }
        }
    }
    identities
}

/// File `outcome` under the identity its branch had before the grant.
pub fn restore_review_identity(
    outcome: &mut WorkflowV2BranchOutcome,
    identities: &BTreeMap<String, String>,
) {
    if let Some(identity) = identities.get(&outcome.item_id) {
        outcome.item_input_hash = Some(identity.clone());
    }
}

/// Mark what shell a review branch ran with.
pub fn mark_review_command_access(result: &mut WorkflowV2Result, shell: bool) {
    if !result.data.is_object() {
        result.data = json!({});
    }
    if let Some(data) = result.data.as_object_mut() {
        data.insert(
            REVIEW_COMMAND_ACCESS_KEY.to_string(),
            Value::String(
                if shell {
                    REVIEW_READ_ONLY_SHELL
                } else {
                    REVIEW_NO_SHELL
                }
                .to_string(),
            ),
        );
    }
}

/// The commands a marked branch view ran, one line each (none for a branch
/// that had no shell); `None` for a branch recorded before the mark.
pub(super) fn branch_commands(view: &Value) -> Option<Vec<Value>> {
    let result = view.get("result")?;
    let access = result
        .get("data")
        .and_then(|data| data.get(REVIEW_COMMAND_ACCESS_KEY))
        .and_then(Value::as_str);
    match access {
        Some(REVIEW_READ_ONLY_SHELL) => {}
        Some(REVIEW_NO_SHELL) => return Some(Vec::new()),
        _ => return None,
    }
    let lines = result
        .get("commands_run")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|record| {
            let command = record.get("command").and_then(Value::as_str)?.trim();
            (!command.is_empty()).then(|| {
                let exit = record
                    .get("exit_code")
                    .and_then(Value::as_i64)
                    .map_or_else(|| "no exit code".to_string(), |code| format!("exit {code}"));
                Value::String(format!("{command} ({exit})"))
            })
        })
        .collect();
    Some(lines)
}

/// `finding` with the commands its branch ran, when the branch was marked.
pub(super) fn stamp_branch_commands(finding: Value, commands: Option<&[Value]>) -> Value {
    match (finding, commands) {
        (Value::Object(mut object), Some(commands)) => {
            object.insert(
                REVIEW_COMMANDS_RUN_KEY.to_string(),
                Value::Array(commands.to_vec()),
            );
            Value::Object(object)
        }
        (finding, _) => finding,
    }
}

#[cfg(test)]
#[path = "review_commands_tests.rs"]
mod tests;
