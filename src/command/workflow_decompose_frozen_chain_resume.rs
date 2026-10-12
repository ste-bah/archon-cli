//! The live frozen-chain decision a resume pins per executor generation.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::WorkflowStore;

use super::freeze_override::{self, OverrideDecision, freeze_override_decision};
use super::{FrozenChainSnapshot, frozen_chain_snapshot, frozen_chain_snapshot_read_only};

const RESUME_CHAIN_DIR: &str = "decomposition/resume-frozen-chain";
const RESUME_CHAIN_SCHEMA: u32 = 1;

/// The live frozen-chain argument plus one `frozen_chain_override` line per
/// phase naming the decision and the record condition that made it.
#[derive(Debug, Clone)]
pub(crate) struct ResumeChainDecision {
    pub(crate) argument: serde_json::Value,
    pub(crate) lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ResumeFrozenChainRecord {
    schema_version: u32,
    generation: u64,
    snapshot: FrozenChainSnapshot,
}

fn resume_chain_path(generation: u64) -> String {
    format!("{RESUME_CHAIN_DIR}/generation-{generation}.json")
}

/// [`resume_chain_decision`] without the reasons.
#[cfg(test)]
pub(crate) fn resume_chain_argument(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    read_only: bool,
) -> Result<serde_json::Value> {
    resume_chain_decision(
        store,
        run_id,
        generation,
        project_root,
        prd_path,
        task_root,
        read_only,
    )
    .map(|decision| decision.argument)
}

/// The launch arguments remain identity-bound. This separate record pins the
/// live snapshot for one executor generation, including runs with no seed;
/// the override decision and its per-phase reasons are derived on each call.
pub(crate) fn resume_chain_decision(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    read_only: bool,
) -> Result<ResumeChainDecision> {
    let relative = resume_chain_path(generation);
    let path = store.run_dir(run_id).join(&relative);
    let recorded = match std::fs::read(&path) {
        Ok(bytes) => {
            let record: ResumeFrozenChainRecord =
                serde_json::from_slice(&bytes).with_context(|| {
                    format!("parsing resume frozen-chain record {}", path.display())
                })?;
            if record.schema_version != RESUME_CHAIN_SCHEMA || record.generation != generation {
                return Err(anyhow!(
                    "resume frozen-chain record {} has schema {} and generation {}; expected schema {} and generation {}",
                    path.display(),
                    record.schema_version,
                    record.generation,
                    RESUME_CHAIN_SCHEMA,
                    generation
                ));
            }
            Some(record.snapshot)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let snapshot = match recorded {
        Some(snapshot) => snapshot,
        None => {
            let snapshot = if read_only {
                frozen_chain_snapshot_read_only(project_root, prd_path, task_root)?
            } else {
                frozen_chain_snapshot(project_root, prd_path, task_root)?
            };
            if !read_only {
                use crate::command::workflow_task_set::{create_dir_all_durably, sync_parent};
                let directory = store.run_dir(run_id).join(RESUME_CHAIN_DIR);
                create_dir_all_durably(&directory)?;
                sync_parent(&directory)?;
                store.write_run_json(
                    run_id,
                    &relative,
                    &ResumeFrozenChainRecord {
                        schema_version: RESUME_CHAIN_SCHEMA,
                        generation,
                        snapshot: snapshot.clone(),
                    },
                )?;
                sync_parent(&path)?;
            }
            snapshot
        }
    };
    // A live frozen artifact is only a reason to skip its authoring phase if
    // the freeze call itself cannot be replayed by this build. In particular,
    // never infer a new phase boundary from the task root alone: normal
    // same-build resumes must replay the exact repair loop they paused in.
    let records = freeze_override::host_command_records(store, run_id)?;
    let decide = |frozen: bool, capability: &str| -> Result<OverrideDecision> {
        if frozen {
            freeze_override_decision(&records, capability)
        } else {
            Ok(OverrideDecision::keep("not frozen on disk"))
        }
    };
    let acceptance = decide(snapshot.acceptance, "freeze-acceptance")?;
    let mut skeleton = decide(snapshot.skeleton, "freeze-skeleton")?;
    // The launcher consumes a chain, so later frozen phases cannot survive a
    // missing earlier phase.
    if skeleton.verify && !acceptance.verify {
        skeleton = OverrideDecision::keep(format!(
            "acceptance is kept and the chain is prefix-closed; own decision: {}",
            skeleton.reason
        ));
    }
    let mut effective = snapshot;
    effective.acceptance = acceptance.verify;
    effective.skeleton = skeleton.verify;
    if !effective.skeleton {
        effective.subjects.clear();
        effective.bodies.clear();
    }
    let lines = [("acceptance", &acceptance), ("skeleton", &skeleton)]
        .into_iter()
        .map(|(phase, decision)| {
            format!(
                "frozen_chain_override {phase}={} reason={}",
                if decision.verify { "verify" } else { "keep" },
                decision.reason
            )
        })
        .collect();
    Ok(ResumeChainDecision {
        argument: effective.to_argument(),
        lines,
    })
}
