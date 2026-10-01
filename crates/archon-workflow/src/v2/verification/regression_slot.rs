//! Batch O2 (CUT-11b): the regression check, run while the script can still
//! act on it.
//!
//! The final regression gate (`regression_gate`) runs after the script has
//! returned: whatever it finds, no round can be planned for it any more.
//! So before the host answers a residual pass's slot checkpoint (pass 2 on;
//! the first pass plans before any residual round has landed) it runs the
//! same comparison -- every declared test command at the run base and at
//! the checkout's HEAD, through the same verdict machinery -- and records
//! what it found, with the files each regressed test lives in, under
//! `baseline-tests/regression-slots/`. That pass's planner then routes each
//! regression (a vanished, hidden or newly failing test, a command with no
//! verdict) to its owner as a host-planned round
//! (`residual_plan::residual_regression`). The post-script gate still runs
//! and stays the final judge.
//!
//! A slot's record is written when its checkpoint is EXECUTED, keyed by the
//! checkpoint's call id, and never rewritten once that checkpoint is
//! recorded: a resumed run replays the checkpoint and reads the same record,
//! so a recorded pass's plan never moves and nothing is re-run.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::regression_compare::{RegressionFinding, compare};
use crate::agent_dispatch_port::WorkflowAgentDispatch;
use crate::task_universe::WorkflowV2TaskUniverse;
use crate::v2::write::test_baseline_run_base::{
    HostRunVerdict, Tree, host_verdicts, run_base_commit,
};
use crate::v2::{WorkflowV2HostCall, WorkflowV2HostMethod, WorkflowV2ResultStore};

const SLOT_DIR: &str = "regression-slots";

/// What one slot's regression check found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionSlot {
    pub call_id: String,
    pub base: Option<String>,
    pub tip: Option<String>,
    /// Every blocking finding, each with the files it implicates.
    pub findings: Vec<SlotFinding>,
    /// Why nothing could be compared, when nothing could.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unjudged: Option<String>,
}

/// One regression and where it lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotFinding {
    pub finding: RegressionFinding,
    /// The test's file at the tip (a vanished test's, at the base).
    pub file: Option<String>,
    /// The files its failure locations name at the tip.
    #[serde(default)]
    pub failure_files: Vec<String>,
}

fn slot_path(store: &WorkflowV2ResultStore, call_id: &str) -> PathBuf {
    let name: String = call_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    store
        .root()
        .join("baseline-tests")
        .join(SLOT_DIR)
        .join(format!("{name}.json"))
}

/// The record a slot's check wrote, if it ran.
pub fn slot_record(store: &WorkflowV2ResultStore, call_id: &str) -> Option<RegressionSlot> {
    let bytes = std::fs::read(slot_path(store, call_id)).ok()?;
    serde_json::from_slice::<RegressionSlot>(&bytes)
        .ok()
        .filter(|slot| slot.call_id == call_id)
}

/// Every declared test command of the universe, trimmed, deduplicated and
/// sorted -- plain runner invocations and every other runner alike.
pub fn declared_commands(universe: &WorkflowV2TaskUniverse) -> Vec<String> {
    universe
        .tasks
        .iter()
        .flat_map(|task| &task.focused_tests)
        .map(|command| command.trim().to_string())
        .filter(|command| !command.is_empty())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Every command's verdict at `commit`: plain ones through the run-base
/// machinery, the rest generically; each cached per (commit, command).
pub(crate) async fn verdicts_at(
    store: &WorkflowV2ResultStore,
    dispatch: &dyn WorkflowAgentDispatch,
    root: &Path,
    commit: &str,
    commands: &[String],
) -> BTreeMap<String, HostRunVerdict> {
    let mut verdicts = host_verdicts(store, dispatch, root, Tree::RunBase, commit, commands).await;
    verdicts.extend(
        super::regression_generic::generic_verdicts(store, dispatch, root, commit, commands).await,
    );
    verdicts
}

/// Run the check for `call` when it is a residual pass's slot (pass 2 on)
/// being executed, and record it; a no-op for every other call, for a slot
/// whose checkpoint is already recorded with its check, and (M3) for a
/// later slot (pass 4 on) of a recording that already moved on past its
/// residual passes -- that pass plans nothing, so nothing is run for it.
/// A record that cannot be written is an error: the slot is not answered
/// on a check the plan could never read.
pub async fn prepare_slot(
    store: &WorkflowV2ResultStore,
    dispatch: &dyn WorkflowAgentDispatch,
    universe: Option<&WorkflowV2TaskUniverse>,
    root: &Path,
    call: &WorkflowV2HostCall,
) -> crate::WorkflowResult<()> {
    let pass = crate::v2::script::residual_plan::slot_pass(call);
    if call.method != WorkflowV2HostMethod::Checkpoint || pass.is_none_or(|pass| pass < 2) {
        return Ok(());
    }
    let Some(universe) = universe else {
        return Ok(());
    };
    let recorded = store.load_call_record(&call.id)?.is_some();
    if recorded && slot_record(store, &call.id).is_some() {
        return Ok(());
    }
    if pass.is_some_and(|pass| pass >= 4)
        && crate::v2::script::residual_plan::recording_moved_on(&store.load_call_records()?, None)
    {
        return Ok(());
    }
    let slot = check(store, dispatch, universe, root, &call.id).await;
    write_slot(store, &slot)
}

/// Record `slot` under its call id, atomically.
pub(crate) fn write_slot(
    store: &WorkflowV2ResultStore,
    slot: &RegressionSlot,
) -> crate::WorkflowResult<()> {
    let path = slot_path(store, &slot.call_id);
    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(slot)?)?;
        std::fs::rename(&tmp, &path)
    };
    write().map_err(|source| crate::WorkflowError::Io {
        path: path.clone(),
        source,
    })
}

async fn check(
    store: &WorkflowV2ResultStore,
    dispatch: &dyn WorkflowAgentDispatch,
    universe: &WorkflowV2TaskUniverse,
    root: &Path,
    call_id: &str,
) -> RegressionSlot {
    let mut slot = RegressionSlot {
        call_id: call_id.to_string(),
        base: run_base_commit(store),
        tip: crate::repository_record::git_head(root).ok(),
        ..RegressionSlot::default()
    };
    let commands = declared_commands(universe);
    let (Some(base), Some(tip)) = (slot.base.clone(), slot.tip.clone()) else {
        slot.unjudged = Some("the run's base commit or the checkout's HEAD is unreadable".into());
        return slot;
    };
    if commands.is_empty() || base == tip {
        return slot;
    }
    let base_runs = verdicts_at(store, dispatch, root, &base, &commands).await;
    let tip_runs = verdicts_at(store, dispatch, root, &tip, &commands).await;
    for finding in compare(&commands, &base_runs, &tip_runs)
        .into_iter()
        .filter(RegressionFinding::blocks)
    {
        let at_tip = tip_runs.get(finding.command());
        let (file, failure_files) = match finding.test() {
            Some(test) => {
                let at = if matches!(finding, RegressionFinding::Vanished { .. }) {
                    &base
                } else {
                    &tip
                };
                let failure_files: Vec<String> = at_tip
                    .and_then(|run| run.failure_files.get(test))
                    .cloned()
                    .unwrap_or_default();
                let file = crate::v2::write::test_baseline_owner_at::test_file_at(
                    root,
                    Some(at),
                    finding.command(),
                    test,
                )
                .or_else(|| failure_files.first().cloned());
                (file, failure_files)
            }
            None => (None, Vec::new()),
        };
        slot.findings.push(SlotFinding {
            finding,
            file,
            failure_files,
        });
    }
    slot
}

#[cfg(test)]
#[path = "regression_slot_tests.rs"]
mod tests;
