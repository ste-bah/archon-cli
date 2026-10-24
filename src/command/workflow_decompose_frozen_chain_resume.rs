//! The live frozen-chain decision a resume pins per executor generation.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::WorkflowStore;

use super::freeze_override::{self, OverrideDecision, freeze_override_decision};
use super::{FrozenChainSnapshot, snapshot_locked, with_chain_read};

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
    let catalog = crate::command::workflow_host_command_catalog::fixed_decomposition_catalog(
        &crate::command::workflow_test_seams::binary_revision(),
    )?;
    resume_chain_decision(
        store,
        run_id,
        generation,
        (project_root, prd_path, task_root),
        &catalog,
        read_only,
    )
    .map(|decision| decision.argument)
}

/// The launch arguments remain identity-bound. This separate record pins the
/// live snapshot for one executor generation, including runs with no seed;
/// the override decision and its per-phase reasons are derived on each call.
/// `catalog` is the one the run's executor judges reuse with.
pub(crate) fn resume_chain_decision(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    (project_root, prd_path, task_root): (&Path, &Path, &Path),
    catalog: &archon_workflow::CommandCapabilityCatalog,
    read_only: bool,
) -> Result<ResumeChainDecision> {
    // Issue 294: the snapshot and every receipt the decisions compare with
    // the disk are read under one hold of the publish lock.
    with_chain_read(project_root, task_root, !read_only, || {
        #[cfg(test)]
        crate::command::workflow_task_set::reader_test_step("resume-chain-read");
        decide_locked(
            store,
            run_id,
            generation,
            (project_root, prd_path, task_root),
            catalog,
            read_only,
        )
    })
}

fn decide_locked(
    store: &WorkflowStore,
    run_id: &str,
    generation: u64,
    (project_root, prd_path, task_root): (&Path, &Path, &Path),
    catalog: &archon_workflow::CommandCapabilityCatalog,
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
            let snapshot = snapshot_locked(project_root, prd_path, task_root)?;
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
    let records = freeze_override::call_records(store, run_id)?;
    let decide = |frozen: bool, capability: &str| -> Result<OverrideDecision> {
        if frozen {
            freeze_override_decision(
                &records,
                catalog,
                capability,
                freeze_override::runs(capability),
            )
        } else {
            Ok(OverrideDecision::keep("not frozen on disk"))
        }
    };
    let acceptance = decide(snapshot.acceptance, "freeze-acceptance")?;
    // The launcher consumes a chain, so later frozen phases cannot survive a
    // missing earlier phase.
    let after = |earlier: &OverrideDecision, name: &str, own: OverrideDecision| {
        if own.verify && !earlier.verify {
            OverrideDecision::keep(format!(
                "{name} is kept and the chain is prefix-closed; own decision: {}",
                own.reason
            ))
        } else {
            own
        }
    };
    let skeleton = after(
        &acceptance,
        "acceptance",
        decide(snapshot.skeleton, "freeze-skeleton")?,
    );
    let mut lines: Vec<String> = [("acceptance", &acceptance), ("skeleton", &skeleton)]
        .into_iter()
        .map(|(phase, decision)| override_line(phase, decision))
        .collect();
    // A body on disk is frozen only as its last judged landing left it: in
    // observe mode a landing publishes with findings its loop still repairs.
    let mut bodies = Vec::new();
    for subject in &snapshot.subjects {
        if !snapshot.bodies.contains(&subject.file_name) {
            continue;
        }
        let own = freeze_override_decision(
            &records,
            catalog,
            &format!("land-task-body for {}", subject.file_name),
            freeze_override::lands_body(&records, subject),
        )?;
        let decision = after(&skeleton, "skeleton", own);
        lines.push(override_line(
            &format!("body:{}", subject.file_name),
            &decision,
        ));
        if decision.verify {
            bodies.push(subject.file_name.clone());
        }
    }
    let mut effective = snapshot;
    effective.acceptance = acceptance.verify;
    effective.skeleton = skeleton.verify;
    effective.bodies = bodies;
    if !effective.skeleton {
        effective.subjects.clear();
    }
    // The verified skeleton's entries come from the published skeleton on
    // every resume, never from the record: a record an older build wrote
    // carries none, and the record itself is never rewritten.
    effective.tasks = if effective.skeleton {
        published_tasks(task_root, &effective)?
    } else {
        Vec::new()
    };
    Ok(ResumeChainDecision {
        argument: effective.to_argument(),
        lines,
    })
}

/// The published skeleton's task entries when it is the skeleton `snapshot`
/// records: the same digest where the record has one, and the same frozen
/// subjects in order. Another skeleton yields none, so the script re-authors
/// every body a skeleton re-author could reach, as without entries.
fn published_tasks(
    task_root: &Path,
    snapshot: &FrozenChainSnapshot,
) -> Result<Vec<archon_workflow::task_skeleton::FrozenTask>> {
    use archon_workflow::task_set_contract::{TASK_SKELETON_FILE, content_digest};
    let path = task_root.join(TASK_SKELETON_FILE);
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading published task skeleton {}", path.display()))?;
    let recorded = (snapshot.preserved_digests.iter()).find(|(file, _)| file == TASK_SKELETON_FILE);
    if recorded.is_some_and(|(_, digest)| *digest != content_digest(&bytes)) {
        return Ok(Vec::new());
    }
    let skeleton: archon_workflow::task_skeleton::TaskSkeleton = serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing published task skeleton {}", path.display()))?;
    let same_subjects = skeleton.tasks.len() == snapshot.subjects.len()
        && (skeleton.tasks.iter().zip(&snapshot.subjects)).all(|(task, subject)| {
            task.task_id == subject.task_id && task.file_name == subject.file_name
        });
    Ok(if same_subjects {
        skeleton.tasks
    } else {
        Vec::new()
    })
}

fn override_line(phase: &str, decision: &OverrideDecision) -> String {
    format!(
        "frozen_chain_override {phase}={} reason={}",
        if decision.verify { "verify" } else { "keep" },
        decision.reason
    )
}
