//! Establishing the test baseline for every branch of one wave.
//!
//! Runs each distinct declared command once per (base commit, command) —
//! from the cache when an earlier pass or wave already ran it, otherwise in
//! the first requesting branch's pristine worktree, at most `parallelism`
//! at a time — then classifies every failure per branch in wave order, so
//! two branches meeting the same unowned file settle on one owner instead
//! of both declaring it and failing the wave's overlap guard.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use archon_write_plan::ForbiddenPaths;
use serde_json::{Value, json};

use super::test_baseline::{
    BaselineObligation, BranchBaseline, CommandBaseline, IgnoredFailure, PreExistingDiagnostics,
    RoutedFailure, SCHEMA_VERSION, cache_command, cached_command, route_finding,
    routed_findings_for_task, save_record,
};
use super::test_baseline_owner::{Ownership, ownership, test_file};
use super::test_baseline_parse::{diagnostic_files, failing_tests, is_cargo_test_command, tail};
use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::WorkflowV2ResultStore;

/// What one branch brings to the wave baseline.
pub(super) struct BranchBaselineRequest {
    pub(super) branch_id: String,
    pub(super) task_ids: Vec<String>,
    /// Declared focused test commands, verbatim.
    pub(super) commands: Vec<String>,
    /// The branch's worktree at the base commit, before any partial work.
    pub(super) worktree: PathBuf,
    /// Repo-relative declared targets.
    pub(super) targets: Vec<String>,
    pub(super) forbidden: ForbiddenPaths,
}

pub(super) struct WaveBaselineContext<'a> {
    pub(super) store: &'a WorkflowV2ResultStore,
    pub(super) dispatch: &'a dyn WorkflowAgentDispatch,
    pub(super) universe: Option<&'a WorkflowV2TaskUniverse>,
    pub(super) stage_id: &'a str,
    pub(super) base_commit: &'a str,
    pub(super) parallelism: usize,
}

/// One [`BranchBaseline`] per request, in request order; each is persisted
/// and every routed failure is queued for its owner before this returns.
pub(super) async fn establish_wave(
    ctx: &WaveBaselineContext<'_>,
    requests: &[BranchBaselineRequest],
) -> Vec<BranchBaseline> {
    let verdicts = verdicts_for(ctx, requests).await;
    let mut taken: BTreeMap<String, String> = BTreeMap::new();
    let mut records: Vec<BranchBaseline> = requests
        .iter()
        .map(|request| classify(ctx, request, &verdicts, &mut taken))
        .collect();
    for record in &records {
        for routed in &record.routed {
            route_finding(
                ctx.store,
                &routed.owner_task,
                finding_for(routed, ctx.base_commit, &record.canonical_task_ids),
            );
        }
    }
    for record in &mut records {
        record.inherited = inherited_for(ctx.store, &record.canonical_task_ids, &record.routed);
        // A branch with nothing declared and nothing routed to it leaves no
        // record: its verifier keeps the rules it had, rather than a stamp
        // that names no test and holds it to a filter it does not have.
        if !record.is_empty() {
            save_record(ctx.store, record);
        }
    }
    records
}

/// Every distinct command's verdict, keyed by the command text.
async fn verdicts_for(
    ctx: &WaveBaselineContext<'_>,
    requests: &[BranchBaselineRequest],
) -> BTreeMap<String, CommandBaseline> {
    let mut pending: Vec<(String, PathBuf)> = Vec::new();
    let mut verdicts = BTreeMap::new();
    for request in requests {
        for command in &request.commands {
            if verdicts.contains_key(command) || pending.iter().any(|(c, _)| c == command) {
                continue;
            }
            match cached_command(ctx.store, ctx.base_commit, command) {
                Some(hit) => {
                    verdicts.insert(command.clone(), hit);
                }
                None => pending.push((command.clone(), request.worktree.clone())),
            }
        }
    }
    if pending.is_empty() {
        return verdicts;
    }
    let semaphore = Arc::new(tokio::sync::Semaphore::new(ctx.parallelism.max(1)));
    let runs = pending.into_iter().map(|(command, worktree)| {
        let semaphore = semaphore.clone();
        async move {
            let _permit = semaphore.acquire_owned().await;
            let run =
                super::test_baseline_run::run_in_worktree(ctx.dispatch, &worktree, &command).await;
            let failing = if is_cargo_test_command(&command) {
                failing_tests(&run.output)
            } else {
                Vec::new()
            };
            let passed = run.exit_code == Some(0) && !run.timed_out && run.error.is_none();
            // A failure no test name explains is attributed by the files its
            // error diagnostics point at (Issue-64).
            let diagnostics = if passed || !failing.is_empty() {
                Vec::new()
            } else {
                diagnostic_files(&run.output, &worktree)
            };
            let verdict = CommandBaseline {
                command: command.clone(),
                base_commit: ctx.base_commit.to_string(),
                exit_code: run.exit_code,
                timed_out: run.timed_out,
                duration_ms: run.duration_ms,
                tail: if passed || !failing.is_empty() {
                    Vec::new()
                } else {
                    tail(&run.output)
                },
                failing_tests: failing,
                error: run.error,
                cached: false,
                diagnostic_files: diagnostics,
            };
            // A timed-out or unstartable command is not cached: the next
            // pass should try again rather than inherit a missing verdict.
            if verdict.error.is_none() {
                cache_command(ctx.store, &verdict);
            }
            (command, verdict)
        }
    });
    for (command, verdict) in futures_util::future::join_all(runs).await {
        verdicts.insert(command, verdict);
    }
    verdicts
}

/// One branch's record from the wave's verdicts. `taken` maps an unowned
/// file to the first task in this wave that took it as an obligation.
fn classify(
    ctx: &WaveBaselineContext<'_>,
    request: &BranchBaselineRequest,
    verdicts: &BTreeMap<String, CommandBaseline>,
    taken: &mut BTreeMap<String, String>,
) -> BranchBaseline {
    let mut record = BranchBaseline {
        schema_version: SCHEMA_VERSION,
        stage_id: ctx.stage_id.to_string(),
        branch_id: request.branch_id.clone(),
        base_commit: ctx.base_commit.to_string(),
        canonical_task_ids: request.task_ids.clone(),
        commands: Vec::new(),
        obligations: Vec::new(),
        routed: Vec::new(),
        ignored: Vec::new(),
        inherited: Vec::new(),
        pre_existing: Vec::new(),
    };
    let own_label = request
        .task_ids
        .first()
        .cloned()
        .unwrap_or_else(|| request.branch_id.clone());
    for command in &request.commands {
        let Some(verdict) = verdicts.get(command) else {
            continue;
        };
        record.commands.push(verdict.clone());
        if verdict.passed() {
            continue;
        }
        if verdict.failed_unattributed() {
            // A failure with no name and a real exit is the task's to clear
            // — unless its diagnostics point only at files outside the
            // task's scope (Issue-64); a missing verdict (timeout, could
            // not start) is recorded only.
            if verdict.exit_code.is_some() {
                place_diagnostics(&mut record, request, ctx.universe, command, verdict);
            }
            continue;
        }
        for test_id in &verdict.failing_tests {
            place(
                &mut record,
                request,
                &own_label,
                ctx.universe,
                command,
                test_id,
                taken,
            );
        }
    }
    record
}

/// A failed command that names no test: its error diagnostics, attributed
/// by file. A file in the task's scope is the task's obligation; the rest
/// are recorded as pre-existing and out of scope, never as obligations.
/// No location at all: the whole failure is the task's, as before.
fn place_diagnostics(
    record: &mut BranchBaseline,
    request: &BranchBaselineRequest,
    universe: Option<&WorkflowV2TaskUniverse>,
    command: &str,
    verdict: &CommandBaseline,
) {
    if verdict.diagnostic_files.is_empty() {
        record.obligations.push(BaselineObligation {
            test_id: None,
            file: None,
            command: command.to_string(),
        });
        return;
    }
    let mut outside = PreExistingDiagnostics {
        command: command.to_string(),
        files: Vec::new(),
        owners: Vec::new(),
    };
    for file in &verdict.diagnostic_files {
        match ownership(universe, &request.task_ids, &request.targets, file) {
            Ownership::Current => record.obligations.push(BaselineObligation {
                test_id: None,
                file: Some(file.clone()),
                command: command.to_string(),
            }),
            Ownership::Other(task) => {
                outside.files.push(file.clone());
                outside.owners.push((file.clone(), task));
            }
            Ownership::Unowned => outside.files.push(file.clone()),
        }
    }
    if !outside.files.is_empty() {
        record.pre_existing.push(outside);
    }
}

fn place(
    record: &mut BranchBaseline,
    request: &BranchBaselineRequest,
    own_label: &str,
    universe: Option<&WorkflowV2TaskUniverse>,
    command: &str,
    test_id: &str,
    taken: &mut BTreeMap<String, String>,
) {
    // No file: nobody's. There is nothing to widen the scope to and no way
    // for the coder to know what to change, so it is listed to ignore rather
    // than made this task's (Issue-73).
    let Some(file) = test_file(&request.worktree, command, test_id) else {
        record.ignored.push(IgnoredFailure {
            test_id: test_id.to_string(),
            file: None,
            reason: "no file could be resolved from its test id".to_string(),
        });
        return;
    };
    let owner = ownership(universe, &request.task_ids, &request.targets, &file);
    let routed_to = |record: &mut BranchBaseline, owner_task: String| {
        record.routed.push(RoutedFailure {
            test_id: test_id.to_string(),
            file: file.clone(),
            owner_task,
            command: command.to_string(),
        });
    };
    match owner {
        Ownership::Other(task) => routed_to(record, task),
        // Forbidden beats "otherwise yours", however the file was reached
        // — a src module, an integration test file, either way the coder
        // cannot edit it (Issue-73).
        _ if request.forbidden.matches(&file) => record.ignored.push(IgnoredFailure {
            test_id: test_id.to_string(),
            file: Some(file.clone()),
            reason: "its file is forbidden to this task and no other task declares it".to_string(),
        }),
        Ownership::Current => record.obligations.push(BaselineObligation {
            test_id: Some(test_id.to_string()),
            file: Some(file.clone()),
            command: command.to_string(),
        }),
        Ownership::Unowned => match taken.get(&file) {
            Some(holder) if holder != own_label => routed_to(record, holder.clone()),
            _ => {
                taken.insert(file.clone(), own_label.to_string());
                record.obligations.push(BaselineObligation {
                    test_id: Some(test_id.to_string()),
                    file: Some(file.clone()),
                    command: command.to_string(),
                });
            }
        },
    }
}

/// The finding queued for the owner task: shaped like a review finding so
/// `remediateFindings` routes it by `canonical_task_ids` unchanged.
fn finding_for(routed: &RoutedFailure, base_commit: &str, reporters: &[String]) -> Value {
    let sha: String = base_commit.chars().take(12).collect();
    json!({
        "id": format!("baseline_regression_{}", super::sanitize_v2_path_segment(&routed.test_id)),
        "canonical_task_ids": [routed.owner_task],
        "attributable_to_task": true,
        "finding_scope": "baseline_regression",
        "severity": "high",
        "title": format!(
            "test `{}` fails on the base commit {sha} in `{}`, a file this task declares",
            routed.test_id, routed.file
        ),
        "description": format!(
            "The host ran `{}` on the base commit {sha} before any task changed the tree and `{}` \
             was already failing. It lives in `{}`, which this task declares in its files expected \
             to change, so it is this task's to make pass; the branch that found it ({}) was told \
             to ignore it. Make the test pass without disabling or deleting it.",
            routed.command,
            routed.test_id,
            routed.file,
            reporters.join(", ")
        ),
        "test_id": routed.test_id,
        "file": routed.file,
        "command": routed.command,
        "base_commit": base_commit,
        "reported_by": reporters,
    })
}

/// Obligations routed to any of `task_ids` by other branches, excluding the
/// ones this very record routed away.
fn inherited_for(
    store: &WorkflowV2ResultStore,
    task_ids: &[String],
    own_routed: &[RoutedFailure],
) -> Vec<BaselineObligation> {
    let mut out: Vec<BaselineObligation> = Vec::new();
    for task in task_ids {
        for finding in routed_findings_for_task(store, task) {
            let test_id = finding.get("test_id").and_then(Value::as_str);
            let file = finding.get("file").and_then(Value::as_str);
            let command = finding
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let (Some(test_id), Some(file)) = (test_id, file) else {
                continue;
            };
            if own_routed.iter().any(|r| r.test_id == test_id)
                || out.iter().any(|o| o.test_id.as_deref() == Some(test_id))
            {
                continue;
            }
            out.push(BaselineObligation {
                test_id: Some(test_id.to_string()),
                file: Some(file.to_string()),
                command: command.to_string(),
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "test_baseline_wave_tests.rs"]
mod tests;
