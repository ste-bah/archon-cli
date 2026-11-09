//! Issue-118: a red test the judged branch cannot write, red the same way
//! since the run began, never refuses its verdict -- and never passes
//! silently either.
//!
//! The base-commit rule reads every test the verifier's own report names
//! failing, from any command it ran. A test the host's baseline never saw
//! (outside the declared filter) is on no exempt list, so an honest verifier
//! that runs a wider command and names what failed is refused over it --
//! even when the test was already red when the run started and lives in a
//! file the branch may not touch. On a live run residual round 3 (write
//! scope: one file) was refused over seven library tests two earlier runs
//! broke; verifiers that ran the same command and named nothing escaped.
//!
//! Nothing here is taken from the agent's report but the commands it ran.
//! For each failed plain test-runner command of an accepted report the HOST
//! runs it twice (`write::test_baseline_run_base`): on the tree the verifier
//! judged, and at the run's base commit. A red test the verifier named is
//! EXCUSED only when all hold:
//!
//! - the host's run on the judged tree names it failing;
//! - the host's run at the run base names it failing with the SAME failure
//!   signature (panic location and message, normalised) -- a test failing
//!   there for another reason (a missing ignored file, a different
//!   assertion) is not the same failure;
//! - its file, and every file its judged failure locations name, lie outside
//!   the branch's writable scope: its tasks' declared files, the item's
//!   targets, the round's granted residual files.
//!
//! A `pre_existing` claim on a runner command is PROVEN only when the host's
//! own judged-tree run of that command failed, every named failure keeps
//! its base signature, and every name is excused or on the stamp's exempt
//! lists; never from a typed list alone. Every other shape keeps the rule exactly as it was: a test in
//! scope, a test green at the run base (a new failure), a test failing
//! differently, a command the host could not run or that never reached a
//! test summary, and every claim on any other command.
//!
//! An excused test is recorded as a host residual gap
//! (`baseline_unowned_red_tests-<digest>`, medium), naming each test and the
//! files its failure implicates -- its own file, its parent module's, and
//! every file its panic points at -- which the second residual pass routes
//! into a round that may write them (`script::residual_second_pass`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::baseline_rule::{BaselineStamp, red_tests};
use super::unowned_paths::BranchScope;
use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::write::test_baseline_owner::{Ownership, ownership};
use crate::v2::write::test_baseline_owner_at::test_file_at;
use crate::v2::write::test_baseline_run_base::{
    FrozenCommands, HostRunVerdict, MAX_COMMANDS_PER_BRANCH, Tree, host_runnable, host_verdicts,
    run_base_commit,
};
use crate::v2::{
    WorkflowV2BranchOutcome, WorkflowV2CommandKind, WorkflowV2CommandStatus, WorkflowV2HostCall,
    WorkflowV2Result, WorkflowV2ResultStore, WorkflowV2Status,
};

/// Id prefix of the host gap naming excused red tests (a flagged copy
/// carries `unowned_path_` in front of it).
pub const UNOWNED_RED_GAP_ID: &str = "baseline_unowned_red_tests";
/// The data key listing them on the branch result.
pub const UNOWNED_RED_DATA_KEY: &str = "baseline_unowned_red_tests";
/// The data key naming the runner commands the host did not run (over the
/// per-branch bound): they excuse nothing.
pub const NOT_RUN_DATA_KEY: &str = "baseline_run_base_not_run";

/// One excused red test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseRedTest {
    pub test_id: String,
    /// Repo-relative file the test lives in.
    pub file: String,
    /// Every file its failure implicates, `file` first.
    pub files: Vec<String>,
    /// The command the host ran on both trees.
    pub command: String,
    pub run_base: String,
}

/// What the host concluded for one branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BranchJudgement {
    pub excused: Vec<BaseRedTest>,
    /// Observed at the judged tree but absent from both base result sets.
    pub unknown: Vec<String>,
    /// Commands with named failures whose base results remain unknown.
    pub unknown_commands: Vec<String>,
    /// Runner commands whose `pre_existing` claim the host's own run proved.
    pub proven: Vec<String>,
    /// Runner commands over the bound, never run.
    pub not_run: Vec<String>,
}

/// Per branch id.
pub type ExcusedRedTests = BTreeMap<String, BranchJudgement>;

pub struct RunBaseRedContext<'a> {
    pub store: &'a WorkflowV2ResultStore,
    pub dispatch: &'a dyn WorkflowAgentDispatch,
    pub universe: Option<&'a WorkflowV2TaskUniverse>,
    /// The checkout the verifiers ran in.
    pub repository_root: &'a Path,
    /// The fanout call: its residual contract's granted files are scope.
    pub call: &'a WorkflowV2HostCall,
    /// The commit the verifiers judged (the checkout's `HEAD` when they
    /// were dispatched).
    pub judged_commit: Option<&'a str>,
}

/// Failed test commands from the verifier's report that are frozen declarations.
fn runner_commands(result: &WorkflowV2Result, frozen: &FrozenCommands) -> Vec<String> {
    let mut commands: Vec<String> = Vec::new();
    for command in result
        .commands_run
        .iter()
        .filter(|c| c.kind == WorkflowV2CommandKind::Test)
        .filter(|c| c.status == WorkflowV2CommandStatus::Failed)
        .filter(|c| !super::baseline_demo::is_baseline_demonstration(result, c))
        .map(|c| c.command.trim().to_string())
        .filter(|c| host_runnable(c, frozen))
    {
        if !commands.contains(&command) {
            commands.push(command);
        }
    }
    commands
}

/// Test failures named in reports of failed commands that the host did not run.
fn unrun_named(result: &WorkflowV2Result, commands: &[String]) -> Vec<String> {
    result
        .commands_run
        .iter()
        .filter(|c| c.status == WorkflowV2CommandStatus::Failed)
        .filter(|c| !super::baseline_demo::is_baseline_demonstration(result, c))
        .filter(|c| !commands.iter().any(|run| run == c.command.trim()))
        .flat_map(|c| crate::v2::write::test_baseline_parse::failing_tests(&c.output_summary))
        .collect()
}

/// The files a host-planned residual round granted the call.
fn granted_files(call: &WorkflowV2HostCall) -> Vec<String> {
    call.options
        .extra
        .get("remediationContract")
        .and_then(|contract| contract.pointer("/residual/files"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect()
}

/// Run what must be run and judge every accepted branch. Nothing is excused
/// or proven without a universe, a recorded run base, a judged commit, or a
/// host-runnable command.
pub async fn excuse_run_base_red_tests(
    ctx: &RunBaseRedContext<'_>,
    outcomes: &[WorkflowV2BranchOutcome],
    by_item: &BTreeMap<String, BaselineStamp>,
    scope_by_item: &BTreeMap<String, BranchScope>,
) -> ExcusedRedTests {
    let mut judged = ExcusedRedTests::new();
    let frozen = FrozenCommands::from_universe(ctx.universe);
    let (Some(universe), Some(base), Some(commit)) =
        (ctx.universe, run_base_commit(ctx.store), ctx.judged_commit)
    else {
        return judged;
    };
    let mut pending: Vec<(&WorkflowV2BranchOutcome, &BaselineStamp, Vec<String>)> = Vec::new();
    for outcome in outcomes.iter().filter(|outcome| {
        matches!(
            outcome.status,
            WorkflowV2Status::Accepted | WorkflowV2Status::Noop
        )
    }) {
        let (Some(stamp), Some(result)) = (by_item.get(&outcome.item_id), outcome.result.as_ref())
        else {
            continue;
        };
        let mut commands = runner_commands(result, &frozen);
        // Nothing to excuse and no runner claim to prove: nothing is run.
        let claims = result
            .commands_run
            .iter()
            .any(|c| c.pre_existing && commands.iter().any(|command| command == c.command.trim()));
        if commands.is_empty() || (red_tests(result, stamp, &[]).is_empty() && !claims) {
            continue;
        }
        let not_run = commands.split_off(commands.len().min(MAX_COMMANDS_PER_BRANCH));
        if !not_run.is_empty() {
            judged.entry(outcome.item_id.clone()).or_default().not_run = not_run;
        }
        pending.push((outcome, stamp, commands));
    }
    let all: Vec<String> = pending
        .iter()
        .flat_map(|(_, _, commands)| commands.iter().cloned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if all.is_empty() {
        return judged;
    }
    let root = ctx.repository_root;
    let on_judged = host_verdicts(
        ctx.store,
        ctx.dispatch,
        root,
        Tree::Judged,
        commit,
        &all,
        &frozen,
    )
    .await;
    let at_base = host_verdicts(
        ctx.store,
        ctx.dispatch,
        root,
        Tree::RunBase,
        &base,
        &all,
        &frozen,
    )
    .await;
    let granted = granted_files(ctx.call);
    for (outcome, stamp, commands) in pending {
        let Some(result) = outcome.result.as_ref() else {
            continue;
        };
        let mut targets = scope_by_item
            .get(&outcome.item_id)
            .map(|scope| scope.targets.clone())
            .unwrap_or_default();
        targets.extend(granted.iter().cloned());
        let scope = Scope {
            universe,
            tasks: &stamp.tasks,
            targets: &targets,
        };
        let mut red = red_tests(result, stamp, &[]);
        red.extend(
            commands
                .iter()
                .filter_map(|command| on_judged.get(command))
                .flat_map(|run| run.failing_tests.iter().cloned()),
        );
        red.sort();
        red.dedup();
        let entry = judged.entry(outcome.item_id.clone()).or_default();
        // A runner the host did not run could name the same test failing
        // another way: with any over the bound, nothing is excused.
        if entry.not_run.is_empty() {
            let unrun = unrun_named(result, &commands);
            entry.excused = classify(
                &red, &unrun, &commands, &on_judged, &at_base, root, commit, &scope,
            );
        }
        if entry.not_run.is_empty() {
            entry.unknown = red
                .iter()
                .filter(|id| {
                    !unrun_named(result, &commands).contains(id)
                        && commands
                            .iter()
                            .filter(|command| {
                                on_judged
                                    .get(*command)
                                    .is_some_and(|run| run.failing_tests.contains(id))
                            })
                            .all(|command| {
                                at_base.get(command).is_some_and(|run| {
                                    !run.passed_tests.contains(id)
                                        && !run.failing_tests.contains(id)
                                })
                            })
                        && commands.iter().any(|command| {
                            on_judged
                                .get(command)
                                .is_some_and(|run| run.failing_tests.contains(id))
                        })
                })
                .cloned()
                .collect();
        }
        let ids: Vec<&str> = entry.excused.iter().map(|t| t.test_id.as_str()).collect();
        for command in &commands {
            if let Some(run) = on_judged.get(command) {
                let named = run.exit_code != Some(0)
                    && !run.failing_tests.is_empty()
                    && run
                        .failed_count
                        .is_none_or(|count| count <= run.failing_tests.len());
                let diagnostics_proven = at_base.get(command).is_some_and(|base| {
                    run.failing_tests.is_empty()
                        && base.exit_code == run.exit_code
                        && run.exit_code != Some(0)
                        && crate::v2::write::test_baseline_parse::identity_subset(
                            base.signatures.get("").map(String::as_str),
                            run.signatures.get("").map(String::as_str),
                        )
                        && run
                            .failure_files
                            .get("")
                            .is_some_and(|files| files.iter().all(|file| !scope.writes(file)))
                });
                let same_identity = at_base.get(command).is_some_and(|base| {
                    base.exit_code == run.exit_code
                        && crate::v2::write::test_baseline_parse::identity_subset(
                            base.signatures.get("").map(String::as_str),
                            run.signatures.get("").map(String::as_str),
                        )
                        && run.failing_tests.iter().all(|test| {
                            base.failing_tests.contains(test)
                                && crate::v2::write::test_baseline_parse::identity_subset(
                                    base.signatures.get(test).map(String::as_str),
                                    run.signatures.get(test).map(String::as_str),
                                )
                        })
                });
                let proven = diagnostics_proven
                    || named
                        && same_identity
                        && run
                            .failing_tests
                            .iter()
                            .all(|t| ids.contains(&t.as_str()) || stamp.exempt(t));
                if proven {
                    entry.proven.push(command.clone());
                } else if named
                    && run.failing_tests.iter().any(|t| entry.unknown.contains(t))
                    && run.failing_tests.iter().all(|t| {
                        ids.contains(&t.as_str()) || stamp.exempt(t) || entry.unknown.contains(t)
                    })
                {
                    entry.unknown_commands.push(command.clone());
                }
            }
        }
    }
    judged.retain(|_, entry| {
        !(entry.excused.is_empty()
            && entry.unknown.is_empty()
            && entry.proven.is_empty()
            && entry.not_run.is_empty())
    });
    judged
}

pub(crate) struct Scope<'a> {
    pub universe: &'a WorkflowV2TaskUniverse,
    pub tasks: &'a [String],
    pub targets: &'a [String],
}

impl Scope<'_> {
    fn writes(&self, file: &str) -> bool {
        ownership(Some(self.universe), self.tasks, self.targets, file) == Ownership::Current
    }
}

/// The red tests the host's own two runs excuse; see the module doc. A test
/// is excused only when EVERY command whose judged run names it failing
/// fails it the same way at the run base, in files outside the scope, and
/// at least one does: one command's match never speaks for another's run.
#[allow(clippy::too_many_arguments)]
pub(crate) fn classify(
    red: &[String],
    unrun: &[String],
    commands: &[String],
    on_judged: &BTreeMap<String, HostRunVerdict>,
    at_base: &BTreeMap<String, HostRunVerdict>,
    root: &Path,
    commit: &str,
    scope: &Scope<'_>,
) -> Vec<BaseRedTest> {
    let mut out: Vec<BaseRedTest> = Vec::new();
    for test_id in red {
        // A failed command the host did not run naming it could be failing
        // it another way: nothing speaks for that run.
        if unrun.iter().any(|named| named == test_id) {
            continue;
        }
        let naming: Vec<&String> = commands
            .iter()
            .filter(|command| {
                on_judged
                    .get(*command)
                    .is_some_and(|run| run.failing_tests.contains(test_id))
            })
            .collect();
        let each: Option<Vec<BaseRedTest>> = naming
            .iter()
            .map(|command| excused_by(command, test_id, on_judged, at_base, root, commit, scope))
            .collect();
        let Some(mut each) = each.filter(|each| !each.is_empty()) else {
            continue;
        };
        let mut first = each.remove(0);
        for other in each {
            for file in other.files {
                if !first.files.contains(&file) {
                    first.files.push(file);
                }
            }
        }
        out.push(first);
    }
    out
}

/// The excusal of `test_id` by one command's two runs, or `None`.
fn excused_by(
    command: &str,
    test_id: &str,
    on_judged: &BTreeMap<String, HostRunVerdict>,
    at_base: &BTreeMap<String, HostRunVerdict>,
    root: &Path,
    commit: &str,
    scope: &Scope<'_>,
) -> Option<BaseRedTest> {
    let (now, then) = (on_judged.get(command)?, at_base.get(command)?);
    let signature = now.signatures.get(test_id)?;
    let same = now.exit_code == then.exit_code
        && then.failing_tests.iter().any(|t| t == test_id)
        && crate::v2::write::test_baseline_parse::identity_subset(
            then.signatures.get(test_id).map(String::as_str),
            Some(signature),
        );
    if !same {
        return None;
    }
    let file = test_file_at(root, Some(commit), command, test_id)
        .or_else(|| now.failure_files.get(test_id)?.first().cloned())?;
    let located = now.failure_files.get(test_id).cloned().unwrap_or_default();
    if scope.writes(&file) || located.iter().any(|path| scope.writes(path)) {
        return None;
    }
    let mut files = vec![file.clone()];
    for extra in located {
        if !files.contains(&extra) {
            files.push(extra);
        }
    }
    Some(BaseRedTest {
        test_id: test_id.to_string(),
        file,
        files,
        command: command.to_string(),
        run_base: then.commit.clone(),
    })
}

#[path = "baseline_run_base_record.rs"]
mod record;
pub(crate) use record::record_unowned_red_tests;
pub use record::{grouped_names, is_unowned_red_gap_id};

#[cfg(test)]
#[path = "baseline_run_base_tests.rs"]
mod tests;
