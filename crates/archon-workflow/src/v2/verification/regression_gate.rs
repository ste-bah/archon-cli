//! Issue-114: the final gate's regression check -- a test that was not
//! failing at the run's base commit and fails at the final tip blocks.
//!
//! Every other gate judges what a verifier or a round said about the work it
//! was given. None compares the repository the run ends on with the one it
//! began on, so a landing that breaks another task's test -- one no later
//! verifier ran -- goes green unseen, and failures that predate the run are
//! indistinguishable from ones it caused.
//!
//! At the final gate the host runs the union of every task's declared test
//! commands itself, through the Issue-118 machinery
//! (`write::test_baseline_run_base`) for plain runner invocations with
//! known options, always `--no-fail-fast`, a verdict only when every test
//! binary reported -- and every other declared command generically
//! (`regression_generic`); once at the run base and once at the final tip, each in
//! a throwaway worktree of its commit (never the live checkout) and cached
//! per commit and command, so a resumed run never re-runs either.
//! Commands are deduplicated and sorted, and EVERY one is run (Batch O: a
//! bound here let a regression in a command past it pass the gate).
//!
//! Per (command, test id):
//!
//! - failing at the tip and not failing at the base -- or the base gave the
//!   command no verdict, so nothing shows it was already red -- is a NEW
//!   failure: it blocks, naming the task that declares the test's file, as
//!   the harness cap exhausted: it is found after the last residual pass;
//! - failing at both is PRE-EXISTING: listed prominently, never blocking;
//! - a command with no verdict at the tip blocks, whatever the base gave,
//!   and failures the harness counted but did not name at the tip block
//!   unless the base counted as many unnamed;
//! - a test that passed at the base and is ignored at the tip blocks (it was
//!   hidden, not fixed), and so does one no longer reported at all (Batch O:
//!   a deleted test used to pass the gate as "maybe renamed"; a renamed test
//!   is still reported under its new name, so only the old one blocks and
//!   the round that renamed it answers for it).
//!
//! A declared command that is not a plain runner invocation is compared
//! too (Batch O2, `regression_generic`): run the same way at the base and
//! the tip, its exit code and the test ids it names compared generically.
//! It used to be a NOT-COMPARED note, so a project whose runner is not
//! cargo could never show it did not regress. The same comparison runs
//! before each residual pass from the second (`regression_slot`), where a
//! regression is routed to its owner as a round; this gate stays the final
//! judge.
//!
//! Every text is the host's; nothing here is task-, file- or domain-specific.

use std::path::Path;

use super::regression_compare::{RegressionFinding, compare};
use super::regression_slot::verdicts_at;
use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::WorkflowV2ResultStore;
use crate::v2::write::test_baseline_run_base::{
    HostRunVerdict, Tree, host_verdicts, run_base_commit,
};

/// What the regression check found: blocking clauses and notes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegressionVerdict {
    pub blocking: Vec<String>,
    pub notes: Vec<String>,
}

/// Where the check runs and what it reads.
pub struct RegressionGate<'a> {
    pub store: &'a WorkflowV2ResultStore,
    pub dispatch: &'a dyn WorkflowAgentDispatch,
    pub universe: Option<&'a WorkflowV2TaskUniverse>,
    pub repository_root: &'a Path,
}

/// The union of every task's declared test commands, sorted -- plain runner
/// invocations and every other runner alike (Batch O2: each is compared).
pub fn declared_test_commands(universe: &WorkflowV2TaskUniverse) -> Vec<String> {
    super::regression_slot::declared_commands(universe)
}

/// Run the regression check.
pub async fn regression_verdict(gate: &RegressionGate<'_>) -> RegressionVerdict {
    let mut verdict = RegressionVerdict::default();
    let Some(universe) = gate.universe else {
        return verdict;
    };
    // Every command a HIGH gap owes a test in runs at the tip too, before
    // anything returns: the residual gate judges those gaps on it
    // (`residual_gate_tip`), so none blocks unjudged.
    run_owed_at_tip(gate).await;
    let commands = declared_test_commands(universe);
    if commands.is_empty() {
        return verdict;
    }
    let Some(base) = run_base_commit(gate.store) else {
        verdict.blocking.push(
            "regression gate cannot judge the run: its base commit is not recorded".to_string(),
        );
        return verdict;
    };
    let Ok(tip) = crate::repository_record::git_head(gate.repository_root) else {
        verdict.blocking.push(format!(
            "regression gate cannot judge the run: HEAD of {} is unreadable",
            gate.repository_root.display()
        ));
        return verdict;
    };
    if tip == base {
        verdict.notes.push(
            "regression gate: nothing landed since the run's base commit, so nothing regressed"
                .to_string(),
        );
        return verdict;
    }
    let at = |sha: &str| sha.chars().take(9).collect::<String>();
    let root = gate.repository_root;
    let base_runs = verdicts_at(gate.store, gate.dispatch, root, &base, &commands).await;
    // The tip too in a throwaway worktree of its commit: never the live
    // checkout, whose uncommitted changes are no part of the tip and whose
    // tree a test run must not write into.
    let tip_runs = verdicts_at(gate.store, gate.dispatch, root, &tip, &commands).await;
    let (base_at, tip_at) = (at(&base), at(&tip));
    let late = "found at the final gate, after the last residual pass (harness cap exhausted: no pass remains to plan its fix)";
    for finding in compare(&commands, &base_runs, &tip_runs) {
        use RegressionFinding as F;
        let owner_of = |command: &str, test: &str| match tip_runs.get(command) {
            Some(run) => owner(gate, universe, &tip, command, test, run),
            None => "no file could be resolved for it".to_string(),
        };
        match &finding {
            F::NoTipVerdict { command, base_had } => verdict.blocking.push(format!(
                "regression gate: `{command}` gave no verdict at the final tip {tip_at} (a build failure, a timeout or a runner that did not report every binary){}; {late}",
                if *base_had {
                    format!(" though it did at the run base {base_at}")
                } else {
                    format!(", nor at the run base {base_at}")
                }
            )),
            F::UnnamedPreExisting { command } => verdict.notes.push(format!(
                "PRE-EXISTING: `{command}` fails at the run base {base_at} and at the final tip {tip_at} with failures its runner does not name"
            )),
            F::NotJudgeable { command, exit_code } => verdict.notes.push(format!(
                "NOT JUDGEABLE: `{command}` fails identically at the run base {base_at} and at the final tip {tip_at} (exit {}) and names no test at either, so the host cannot tell whether the run regressed it",
                exit_code.map_or_else(|| "none".to_string(), |code| code.to_string())
            )),
            F::UnnamedNew { command } => verdict.blocking.push(format!(
                "regression gate: `{command}` fails at the final tip {tip_at} with failures its runner does not name, which the run base {base_at} did not; {late}"
            )),
            F::OldBaseCache { command } => verdict.notes.push(format!(
                "warning: `{command}`'s base verdict was cached before passed ids were kept, so a test hidden at the final tip {tip_at} is not detected for it"
            )),
            F::Hidden { command, test } => verdict.blocking.push(format!(
                "regression: `{test}` (`{command}`) passed at the run base {base_at} and is ignored at the final tip {tip_at}; {}; {late}",
                owner_of(command, test)
            )),
            F::Vanished { command, test } => verdict.blocking.push(format!(
                "regression: `{test}` (`{command}`) passed at the run base {base_at} and is not reported at the final tip {tip_at}: it was removed or renamed, and a test that is gone proves nothing passes; {}; {late}",
                owner_of(command, test)
            )),
            F::PreExisting {
                command,
                test,
                moved,
            } => verdict.notes.push(format!(
                "PRE-EXISTING: `{test}` (`{command}`) fails at the run base {base_at} and at the final tip {tip_at}{}",
                if *moved { ", with a different failure" } else { "" }
            )),
            F::NewFailure {
                command,
                test,
                base_had,
            } => {
                let why = if *base_had {
                    format!("did not fail at the run base {base_at}")
                } else {
                    format!("the run base {base_at} gave no verdict to show it already failed")
                };
                verdict.blocking.push(format!(
                    "regression: `{test}` (`{command}`) fails at the final tip {tip_at} and {why}; {}; {late}",
                    owner_of(command, test)
                ));
            }
        }
    }
    verdict
}

/// Run, at the tip in a throwaway worktree (cached), every command a
/// recorded HIGH gap owes a test in (`residual_plan::tip_owed_commands`);
/// returns the commands.
pub async fn run_owed_at_tip(gate: &RegressionGate<'_>) -> Vec<String> {
    let owed =
        crate::v2::script::residual_plan::tip_owed_commands(gate.store, Some(gate.repository_root));
    if !owed.is_empty()
        && let Ok(tip) = crate::repository_record::git_head(gate.repository_root)
    {
        host_verdicts(
            gate.store,
            gate.dispatch,
            gate.repository_root,
            Tree::RunBase,
            &tip,
            &owed,
        )
        .await;
    }
    owed
}

/// Who answers for a new failure: the tasks declaring the test's file.
fn owner(
    gate: &RegressionGate<'_>,
    universe: &WorkflowV2TaskUniverse,
    tip: &str,
    command: &str,
    test: &str,
    run: &HostRunVerdict,
) -> String {
    let file = crate::v2::write::test_baseline_owner_at::test_file_at(
        gate.repository_root,
        Some(tip),
        command,
        test,
    )
    .or_else(|| {
        run.failure_files
            .get(test)
            .and_then(|files| files.first().cloned())
    });
    let Some(file) = file else {
        return "no file could be resolved for it".to_string();
    };
    let owners = crate::v2::script::residual_paths::owners(universe, &file, gate.repository_root);
    if owners.is_empty() {
        format!("no task declares {file}")
    } else {
        format!(
            "{file} is declared by {}, which answers for it",
            owners.into_iter().collect::<Vec<_>>().join(", ")
        )
    }
}

#[cfg(test)]
#[path = "regression_gate_tests.rs"]
mod tests;
