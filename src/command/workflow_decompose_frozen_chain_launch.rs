//! A fresh launch on a frozen acceptance contract (Issue 387, item 3).
//!
//! The launcher builds on a contract it finds frozen: the script verifies it
//! in place (`verify-frozen-acceptance`) and never asks the freeze judge
//! again. That verification checks the bundle, the PRD digest and the stored
//! verdicts, not the rules the freeze applied. When the freeze that published
//! the live contract was judged by logic this build would not replay
//! (Issue 361's test, `recorded_logic_holds`, exactly as a resume asks it in
//! `freeze_override_decision`), or no recorded freeze in this project
//! vouches for the contract at all, the launch verifies every frozen check
//! under this build's acceptance entry rules (the freeze's own shape and
//! live-root rules, `entry_defects`). A check they refuse is re-authored
//! before the run starts, through the sanctioned per-check repair
//! `freeze-acceptance --reauthor` uses (`workflow_decompose_launch_repair`); every other frozen check
//! stays frozen and no judge re-runs on it. Every decision is printed,
//! logged and, for a repair, recorded as an event.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::WorkflowStore;
use archon_workflow::task_set_contract::{ACCEPTANCE_CONTRACT_FILE, AcceptanceContract};

use super::freeze_override::{call_records, freeze_override_decision, runs};

const FREEZE_ACCEPTANCE: &str = "freeze-acceptance";

/// What the project's records say of the freeze that published the live
/// contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordedFreezeLogic {
    /// A recorded freeze accepted it, and this build's logic replays it.
    Holds(String),
    /// A recorded freeze accepted it under logic this build would not replay.
    Stale(String),
    /// No recorded freeze of this project vouches for the live contract.
    Unrecorded(String),
}

/// The recorded freeze of the live contract under `task_root`, judged by
/// the same predicate a resume uses, over every run of `store` that
/// decomposed into this task root.
pub(crate) fn recorded_freeze_logic(
    store: &WorkflowStore,
    task_root: &Path,
    catalog: &archon_workflow::CommandCapabilityCatalog,
) -> Result<RecordedFreezeLogic> {
    let root = task_root.to_string_lossy().replace('\\', "/");
    let (mut stale, mut unrecorded) = (None, Vec::new());
    for run in store.list_runs()? {
        let state = store.run_dir(&run.id).join("decomposition/state.json");
        let Ok(bytes) = std::fs::read(&state) else {
            continue;
        };
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing {}", state.display()))?;
        if value["identity"]["task_root_identity"].as_str() != Some(root.as_str()) {
            continue;
        }
        let records = call_records(store, &run.id)?;
        let decision = freeze_override_decision(
            &records,
            catalog,
            FREEZE_ACCEPTANCE,
            runs(FREEZE_ACCEPTANCE),
        )?;
        let reason = format!("run {}: {}", run.id, decision.reason);
        match decision.logic_holds {
            Some(true) => return Ok(RecordedFreezeLogic::Holds(reason)),
            Some(false) => stale = stale.or(Some(reason)),
            None => unrecorded.push(reason),
        }
    }
    Ok(match stale {
        Some(reason) => RecordedFreezeLogic::Stale(reason),
        None if unrecorded.is_empty() => RecordedFreezeLogic::Unrecorded(
            "no run of this project recorded a freeze-acceptance for this task root".into(),
        ),
        None => RecordedFreezeLogic::Unrecorded(unrecorded.join("; ")),
    })
}

/// What a launch does with an acceptance stage it found frozen.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LaunchAcceptance {
    /// Every decision line, printed and written to the decomposition log.
    pub(crate) lines: Vec<String>,
    /// The frozen checks this build's acceptance entry rules refuse, each
    /// with every refusal text: they are re-authored before the run starts.
    pub(crate) reauthor: BTreeMap<String, String>,
    /// Why the checks were validated at all: the recorded freeze logic.
    pub(crate) recorded: String,
}

/// The launch's decision for an acceptance stage it found `frozen` (nothing
/// otherwise). A check this build's rules refuse is never a reason to stop:
/// it is routed to re-authoring (`workflow_decompose_launch_repair`) and every
/// other frozen check stays frozen. `repository_root`, the project and the
/// task set's own roots are the live roots a check must not name.
pub(crate) fn launch_acceptance_decision(
    frozen: bool,
    store: &WorkflowStore,
    (project_root, task_root, repository_root): (&Path, &Path, &Path),
    catalog: &archon_workflow::CommandCapabilityCatalog,
) -> Result<LaunchAcceptance> {
    if !frozen {
        return Ok(LaunchAcceptance::default());
    }
    let (kind, reason) = match recorded_freeze_logic(store, task_root, catalog)? {
        RecordedFreezeLogic::Holds(reason) => {
            return Ok(LaunchAcceptance {
                lines: vec![format!(
                    "frozen_chain_launch acceptance=verify reason={reason}"
                )],
                ..LaunchAcceptance::default()
            });
        }
        RecordedFreezeLogic::Stale(reason) => ("stale", reason),
        RecordedFreezeLogic::Unrecorded(reason) => ("unrecorded", reason),
    };
    let path = task_root.join(ACCEPTANCE_CONTRACT_FILE);
    let contract: AcceptanceContract = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?,
    )
    .with_context(|| format!("parsing {}", path.display()))?;
    let mut roots: Vec<PathBuf> = vec![repository_root.to_path_buf(), project_root.to_path_buf()];
    roots.extend(
        crate::command::workflow_task_set::live_root::task_set_roots(project_root, task_root)
            .map_err(|why| anyhow!("live roots of {}: {why}", task_root.display()))?,
    );
    let roots = crate::command::workflow_task_set::live_root::absolute_roots(&roots)
        .map_err(|why| anyhow!("live roots of {}: {why}", task_root.display()))?;
    let mut reauthor: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let entries = contract.acceptance.iter().chain(&contract.supplementary);
    for entry in entries.clone() {
        let text = serde_json::to_string(entry)?;
        let defects = crate::command::workflow::entry_defects(&text, &roots)
            .map_err(|why| anyhow!("frozen check '{}' cannot be validated: {why}", entry.id))?;
        for defect in defects {
            let refusal = crate::command::workflow::refusal_text(&entry.id, &defect);
            reauthor.entry(entry.id.clone()).or_default().push(refusal);
        }
    }
    let recorded = format!("{kind} recorded freeze logic ({reason})");
    let total = entries.count();
    let line = if reauthor.is_empty() {
        format!(
            "frozen_chain_launch acceptance=verify reason={recorded}; this build's acceptance entry rules pass all {total} frozen check(s)"
        )
    } else {
        format!(
            "frozen_chain_launch acceptance=reauthor ids={} kept={} reason={recorded}; this build's acceptance entry rules refuse them: {}",
            reauthor.keys().cloned().collect::<Vec<_>>().join(","),
            total - reauthor.len(),
            reauthor
                .values()
                .flatten()
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ")
        )
    };
    Ok(LaunchAcceptance {
        lines: vec![line],
        reauthor: reauthor
            .into_iter()
            .map(|(id, texts)| (id, texts.join("\n")))
            .collect(),
        recorded,
    })
}

#[cfg(test)]
#[path = "workflow_decompose_frozen_chain_launch_tests.rs"]
mod tests;
