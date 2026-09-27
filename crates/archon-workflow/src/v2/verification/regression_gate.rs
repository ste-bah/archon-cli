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
//! (`write::test_baseline_run_base`): only plain runner invocations with
//! known options, always `--no-fail-fast`, a verdict only when every test
//! binary reported; once at the run base (a throwaway worktree, cached per
//! commit and command, so a resumed run never re-runs it) and once at the
//! final tip (in place, only while `HEAD` is the tip; cached per tip).
//! Commands are deduplicated and sorted, and at most
//! [`MAX_REGRESSION_COMMANDS`] are run; the rest are named in a warning.
//!
//! Per (command, test id):
//!
//! - failing at the tip and not failing at the base -- or the base gave the
//!   command no verdict, so nothing shows it was already red -- is a NEW
//!   failure: it blocks, naming the task that declares the test's file, as
//!   the harness cap exhausted: it is found after the last residual pass;
//! - failing at both is PRE-EXISTING: listed prominently, never blocking;
//! - a command with no verdict at the tip blocks unless the base gave none
//!   either, and failures the harness counted but did not name at the tip
//!   block unless the base counted as many unnamed.
//!
//! Every text is the host's; nothing here is task-, file- or domain-specific.

use std::collections::BTreeSet;
use std::path::Path;

use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::WorkflowV2ResultStore;
use crate::v2::write::test_baseline_run_base::{
    HostRunVerdict, Tree, host_runnable, host_verdicts, run_base_commit,
};

/// Most distinct declared commands the gate runs; the rest are named.
pub const MAX_REGRESSION_COMMANDS: usize = 16;

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

/// The union of every task's declared, host-runnable test commands, sorted.
pub fn declared_test_commands(universe: &WorkflowV2TaskUniverse) -> Vec<String> {
    universe
        .tasks
        .iter()
        .flat_map(|task| &task.focused_tests)
        .map(|command| command.trim().to_string())
        .filter(|command| host_runnable(command))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Run the regression check.
pub async fn regression_verdict(gate: &RegressionGate<'_>) -> RegressionVerdict {
    let mut verdict = RegressionVerdict::default();
    let Some(universe) = gate.universe else {
        return verdict;
    };
    let mut commands = declared_test_commands(universe);
    if commands.is_empty() {
        return verdict;
    }
    let total = commands.len();
    let skipped = commands.split_off(total.min(MAX_REGRESSION_COMMANDS));
    if !skipped.is_empty() {
        verdict.notes.push(format!(
            "warning: the regression gate compared {MAX_REGRESSION_COMMANDS} of {total} declared test commands; not compared: {}",
            skipped.join("; ")
        ));
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
    let base_runs = host_verdicts(
        gate.store,
        gate.dispatch,
        gate.repository_root,
        Tree::RunBase,
        &base,
        &commands,
    )
    .await;
    let tip_runs = host_verdicts(
        gate.store,
        gate.dispatch,
        gate.repository_root,
        Tree::Judged,
        &tip,
        &commands,
    )
    .await;
    let (base_at, tip_at) = (at(&base), at(&tip));
    let late = "found at the final gate, after the last residual pass (harness cap exhausted: no pass remains to plan its fix)";
    for command in &commands {
        let at_base = base_runs.get(command);
        let Some(at_tip) = tip_runs.get(command) else {
            match at_base {
                Some(_) => verdict.blocking.push(format!(
                    "regression gate: `{command}` gave no verdict at the final tip {tip_at} (a build failure, a timeout or a runner that did not report every binary) though it did at the run base {base_at}; {late}"
                )),
                None => verdict.notes.push(format!(
                    "PRE-EXISTING: `{command}` gave no verdict at the run base {base_at} nor at the final tip {tip_at}"
                )),
            }
            continue;
        };
        let unnamed = |run: &HostRunVerdict| {
            let counted = run.failed_count.unwrap_or(0);
            let named = run.failing_tests.len();
            if run.exit_code != Some(0) && counted == 0 && named == 0 {
                1
            } else {
                counted.saturating_sub(named)
            }
        };
        let tip_unnamed = unnamed(at_tip);
        if tip_unnamed > 0 {
            match at_base.map(unnamed) {
                Some(before) if before >= tip_unnamed => verdict.notes.push(format!(
                    "PRE-EXISTING: `{command}` fails at the run base {base_at} and at the final tip {tip_at} with failures its runner does not name"
                )),
                _ => verdict.blocking.push(format!(
                    "regression gate: `{command}` fails at the final tip {tip_at} with failures its runner does not name, which the run base {base_at} did not; {late}"
                )),
            }
        }
        let before: BTreeSet<&String> = at_base
            .map(|run| run.failing_tests.iter().collect())
            .unwrap_or_default();
        for test in &at_tip.failing_tests {
            if before.contains(test) {
                let moved = at_base
                    .and_then(|run| run.signatures.get(test))
                    .is_some_and(|sig| at_tip.signatures.get(test) != Some(sig));
                verdict.notes.push(format!(
                    "PRE-EXISTING: `{test}` (`{command}`) fails at the run base {base_at} and at the final tip {tip_at}{}",
                    if moved { ", with a different failure" } else { "" }
                ));
                continue;
            }
            let why = if at_base.is_some() {
                format!("did not fail at the run base {base_at}")
            } else {
                format!("the run base {base_at} gave no verdict to show it already failed")
            };
            verdict.blocking.push(format!(
                "regression: `{test}` (`{command}`) fails at the final tip {tip_at} and {why}; {}; {late}",
                owner(gate, universe, &tip, command, test, at_tip)
            ));
        }
    }
    verdict
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
