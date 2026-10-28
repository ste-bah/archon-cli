//! What a task root already holds of the frozen chain, verified (Issue-46).
//!
//! A decomposition run is pinned to its starting binary, so a run that died
//! at the set gate cannot be resumed by a fixed binary; before this, a fresh
//! launch on the same task root re-authored eight hours of acceptance,
//! skeleton and bodies to reach the same gate. The launcher now reads the
//! root once, verifies whatever chain is frozen there with the SAME lock and
//! pin checks the freeze and lint commands use, and hands the script a
//! `frozenChain` argument saying what it may skip. Nothing here is a new
//! verifier: acceptance is `validate_acceptance_bundle` plus the PRD digest
//! the postcondition checks, the skeleton is `validate_full_chain`.
//!
//! A lock that exists but does not verify is an error at launch, never a
//! silent `false`: the operator asked to build on this root, and a root whose
//! frozen record contradicts its contents is not one to build on quietly.
//! Authenticated authored defects retain admission metadata so verification
//! can return them to their owner; provenance contradictions remain errors.

use std::path::Path;

use anyhow::{Context, Result, anyhow};
use archon_workflow::HostCommandSubject;
use archon_workflow::obligation_ids::acceptance_ids;
use archon_workflow::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE, AcceptanceContract, AcceptancePin,
    TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest, validate_acceptance_bundle,
};
use archon_workflow::task_skeleton::validate_full_chain;

pub(crate) struct LaunchSnapshot {
    pub(crate) frozen_chain: FrozenChainSnapshot,
    pub(crate) decision: LaunchAcceptance,
    pub(crate) deferred: bool,
}

/// Inspect a launch chain without settling a pending publication before claim.
pub(crate) fn prepare_launch_snapshot(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    store: &archon_workflow::WorkflowStore,
    repository_root: &Path,
    catalog: &archon_workflow::CommandCapabilityCatalog,
) -> Result<LaunchSnapshot> {
    let read =
        frozen_chain_snapshot_read_only(project_root, prd_path, task_root).or_else(|error| {
            if verification::is_authored(&error) {
                with_chain_read(project_root, task_root, false, || {
                    snapshot_admission_locked(project_root, prd_path, task_root)
                })
            } else {
                Err(error)
            }
        });
    let deferred = read.as_ref().err().is_some_and(|error| {
        verification::missing_skeleton(error)
            || format!("{error:#}").contains("publish recovery is pending")
    });
    let frozen_chain = match read {
        Ok(snapshot) => snapshot,
        Err(_) if deferred => FrozenChainSnapshot::default(),
        Err(error) => return Err(error),
    };
    let decision = launch_acceptance_decision(
        !deferred && frozen_chain.acceptance,
        store,
        (project_root, task_root, repository_root),
        catalog,
    )?;
    Ok(LaunchSnapshot {
        frozen_chain,
        decision,
        deferred,
    })
}

/// Settle and decide a deferred launch chain after its task root was claimed.
pub(crate) fn recover_deferred_launch_snapshot(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    store: &archon_workflow::WorkflowStore,
    repository_root: &Path,
    catalog: &archon_workflow::CommandCapabilityCatalog,
) -> Result<(FrozenChainSnapshot, LaunchAcceptance)> {
    let snapshot = with_chain_read(project_root, task_root, true, || {
        skeleton_cache::restore_missing(project_root, task_root)?;
        let snapshot = snapshot_admission_locked(project_root, prd_path, task_root)?;
        if snapshot.skeleton {
            skeleton_cache::backfill(project_root, task_root)?;
        }
        Ok(snapshot)
    })?;
    let decision = launch_acceptance_decision(
        snapshot.acceptance,
        store,
        (project_root, task_root, repository_root),
        catalog,
    )?;
    Ok((snapshot, decision))
}

#[path = "workflow_decompose_frozen_chain_override.rs"]
mod freeze_override;
#[cfg(test)]
use freeze_override::{freeze_outcome_decision, freeze_record_status_allows_override};
#[path = "workflow_decompose_frozen_chain_launch.rs"]
mod launch_chain;
pub(crate) use launch_chain::{LaunchAcceptance, launch_acceptance_decision};
#[path = "workflow_decompose_frozen_chain_resume.rs"]
mod resume_chain;
#[cfg(test)]
pub(crate) use resume_chain::resume_chain_argument;
pub(crate) use resume_chain::resume_chain_decision;
#[path = "workflow_decompose_frozen_chain_stage.rs"]
mod stage;
#[cfg(test)]
pub(crate) use stage::stage_verify;
pub(crate) use stage::{FrozenStage, handle_staged_verify};
#[path = "workflow_decompose_frozen_chain_cache.rs"]
pub(crate) mod skeleton_cache;
#[path = "workflow_decompose_frozen_chain_verification.rs"]
mod verification;

/// The frozen chain the launcher found and verified under a task root. Read
/// strictly on resume: a new field follows the rule of
/// `FIXED_DECOMPOSITION_STATE_SCHEMA_VERSION`.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FrozenChainSnapshot {
    pub(crate) acceptance: bool,
    pub(crate) skeleton: bool,
    pub(crate) subjects: Vec<HostCommandSubject>,
    /// Frozen file names whose body file exists under the task root.
    pub(crate) bodies: Vec<String>,
    /// Content digests let launch repair prove that it left frozen bodies intact.
    #[serde(default)]
    pub(crate) body_digests: Vec<(String, String)>,
    /// Frozen artifacts the acceptance repair is not allowed to republish.
    #[serde(default)]
    pub(crate) preserved_digests: Vec<(String, String)>,
    /// The frozen skeleton's task entries as published: a skeleton re-author
    /// re-authors only the bodies whose entry it changes (O3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tasks: Vec<archon_workflow::task_skeleton::FrozenTask>,
}

impl FrozenChainSnapshot {
    /// The script argument: camelCase subjects, exactly as `HostCommandSubject`
    /// serialises for a host outcome, and a frozen skeleton's task entries.
    pub(crate) fn to_argument(&self) -> serde_json::Value {
        let mut argument = serde_json::json!({
            "acceptance": self.acceptance,
            "skeleton": self.skeleton,
            "subjects": self.subjects,
            "bodies": self.bodies,
        });
        if self.skeleton && !self.tasks.is_empty() {
            argument["tasks"] = serde_json::json!(self.tasks);
        }
        argument
    }
}

/// Read and verify the frozen chain under `task_root`. An absent or empty root
/// is nothing frozen; a contract or skeleton without its lock is not frozen
/// either (the freeze command has not published it); a lock is verified.
pub(crate) fn frozen_chain_snapshot(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
) -> Result<FrozenChainSnapshot> {
    frozen_chain_snapshot_with_recovery(project_root, prd_path, task_root, true)
}

/// Read and verify without settling an interrupted publication.
pub(crate) fn frozen_chain_snapshot_read_only(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
) -> Result<FrozenChainSnapshot> {
    if verification::is_directory(task_root)? {
        let pin_path = super::workflow_task_set::acceptance_pin_path(project_root, task_root);
        let lock_path = archon_workflow::task_set_publish_lock::lock_path(&pin_path);
        if !lock_path.try_exists()? {
            let [journal, _] = archon_workflow::task_set_publish_lock::journal_paths(&pin_path);
            if journal.try_exists()? {
                return Err(anyhow!(
                    "frozen task chain publish recovery is pending at {}",
                    journal.display()
                ));
            }
            return snapshot_locked(project_root, prd_path, task_root);
        }
    }
    frozen_chain_snapshot_with_recovery(project_root, prd_path, task_root, false)
}

fn frozen_chain_snapshot_with_recovery(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    recover: bool,
) -> Result<FrozenChainSnapshot> {
    with_chain_read(project_root, task_root, recover, || {
        if recover {
            skeleton_cache::restore_missing(project_root, task_root)?;
        }
        let snapshot = snapshot_locked(project_root, prd_path, task_root)?;
        if recover && snapshot.skeleton {
            skeleton_cache::backfill(project_root, task_root)?;
        }
        Ok(snapshot)
    })
}

/// Read under the chain lock; recovery callers settle interrupted publishes first.
pub(crate) fn with_chain_read<T>(
    project_root: &Path,
    task_root: &Path,
    recover: bool,
    read: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if !verification::is_directory(task_root)? {
        return read();
    }
    let pin_path = super::workflow_task_set::acceptance_pin_path(project_root, task_root);
    if recover {
        let _settled = super::workflow_task_set::lock_and_recover(&pin_path, task_root)?;
        return read();
    }
    let _shared =
        archon_workflow::task_set_publish_lock::PublishLockFile::hold_shared_existing(&pin_path)
            .map_err(|error| anyhow!(error))?;
    let [journal, _] = archon_workflow::task_set_publish_lock::journal_paths(&pin_path);
    if journal.try_exists()? {
        return Err(anyhow!(
            "dry-run cannot inspect frozen task chain while publish recovery is pending at {}; run a live resume to settle it",
            journal.display()
        ));
    }
    read()
}

pub(crate) fn snapshot_locked(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
) -> Result<FrozenChainSnapshot> {
    snapshot_inner(project_root, prd_path, task_root, false)
}

fn snapshot_admission_locked(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
) -> Result<FrozenChainSnapshot> {
    snapshot_inner(project_root, prd_path, task_root, true)
}

fn snapshot_inner(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    admission: bool,
) -> Result<FrozenChainSnapshot> {
    let mut snapshot = FrozenChainSnapshot::default();
    if !verification::is_directory(task_root)? {
        return Ok(snapshot);
    }
    #[cfg(test)]
    super::workflow_task_set::reader_test_step("decomposition-read");
    let acceptance_lock = task_root.join(ACCEPTANCE_LOCK_FILE);
    let skeleton_lock = task_root.join(TASK_SKELETON_LOCK_FILE);
    if !acceptance_lock.try_exists()? {
        if skeleton_lock.try_exists()? {
            return Err(anyhow!(
                "{} exists under {} without {}; the frozen chain is inconsistent — restore the acceptance freeze or clear the task root",
                TASK_SKELETON_LOCK_FILE,
                task_root.display(),
                ACCEPTANCE_LOCK_FILE
            ));
        }
        return Ok(snapshot);
    }
    let pin = read_pin(project_root, task_root)?;
    if let Err(error) = verify_acceptance(project_root, prd_path, task_root, &pin)
        && !(admission && verification::is_authored(&error))
    {
        return Err(error);
    }
    snapshot.acceptance = true;
    if !skeleton_lock.try_exists()? {
        // Originating acceptance-only behavior: without the skeleton lock,
        // the skeleton is not a frozen successor, even if its draft bytes or
        // pin fields remain. Preflight tells operators to remove successor
        // artifacts, and every chain reader therefore leaves this phase open.
        return Ok(snapshot);
    }
    // An acceptance published before the skeleton phase leaves the old
    // skeleton pending. Its lock must still authenticate its exact bytes.
    if super::workflow_skeleton_pending::is_pending(task_root, &pin)? {
        return Ok(snapshot);
    }
    for name in [TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE] {
        verification::read(&task_root.join(name), true)?;
    }
    let skeleton = match validate_full_chain(task_root, &pin) {
        Ok(skeleton) => skeleton,
        Err(error) if admission && !error.defects.is_empty() => verification::admission_skeleton(
            &verification::read(&task_root.join(TASK_SKELETON_FILE), true)?,
        )?,
        Err(error) => {
            return Err(anyhow::Error::new(error).context(format!(
                "frozen task skeleton under {} does not verify",
                task_root.display()
            )));
        }
    };
    snapshot.skeleton = true;
    for file_name in [TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE] {
        let path = task_root.join(file_name);
        let bytes = verification::read(&path, true)
            .with_context(|| format!("reading frozen task-chain artifact {}", path.display()))?;
        snapshot
            .preserved_digests
            .push((file_name.to_string(), content_digest(&bytes)));
    }
    for task in &skeleton.tasks {
        let path = task_root.join(&task.file_name);
        // Invalid authored paths may be retained as comparison inputs, but
        // admission never reads a body outside the owning artifact's directory.
        let direct = Path::new(&task.file_name).components().count() == 1;
        let present = match if direct {
            std::fs::symlink_metadata(&path)
        } else {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        } {
            Ok(meta) => meta.is_file(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading frozen body metadata {}", path.display()));
            }
        };
        if present {
            snapshot.bodies.push(task.file_name.clone());
            let bytes = verification::read(&path, false)
                .with_context(|| format!("reading frozen task body {}", path.display()))?;
            snapshot
                .body_digests
                .push((task.file_name.clone(), content_digest(&bytes)));
            snapshot
                .preserved_digests
                .push((task.file_name.clone(), content_digest(&bytes)));
        }
        snapshot.subjects.push(HostCommandSubject {
            task_id: task.task_id.clone(),
            file_name: task.file_name.clone(),
        });
    }
    snapshot.tasks = skeleton.tasks;
    Ok(snapshot)
}

/// Verify one frozen stage the way the launcher did, for the
/// `verify-frozen-chain` host command. A stage that is not frozen is an
/// error: the script only asks about a stage the launcher reported frozen.
/// Acceptance is read independently of its successor so a repairable successor
/// defect does not prevent verification of its unchanged predecessor.
fn verify_frozen_stage_with_recovery(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    stage: FrozenStage,
    recover: bool,
) -> Result<FrozenChainSnapshot> {
    if stage == FrozenStage::Acceptance {
        return with_chain_read(project_root, task_root, recover, || {
            let pin = read_pin(project_root, task_root)?;
            if let Err(error) = verify_acceptance(project_root, prd_path, task_root, &pin) {
                if verification::is_authored(&error) && pin.skeleton_digest.is_some() {
                    skeleton_cache::backfill(project_root, task_root)?;
                }
                return Err(error);
            }
            Ok(FrozenChainSnapshot {
                acceptance: true,
                ..Default::default()
            })
        });
    }
    let snapshot = frozen_chain_snapshot_with_recovery(project_root, prd_path, task_root, recover)?;
    let frozen = match stage {
        FrozenStage::Acceptance => snapshot.acceptance,
        FrozenStage::Skeleton => snapshot.skeleton,
    };
    if !frozen {
        return Err(anyhow!(
            "no frozen {} under {}; the launch reported one, so the task root changed underneath the run",
            match stage {
                FrozenStage::Acceptance => "acceptance contract",
                FrozenStage::Skeleton => "task skeleton",
            },
            task_root.display()
        ));
    }
    Ok(snapshot)
}

fn read_pin(project_root: &Path, task_root: &Path) -> Result<AcceptancePin> {
    let path = super::workflow_task_set::acceptance_pin_path(project_root, task_root);
    let bytes = verification::read(&path, true).with_context(|| {
        format!(
            "{} exists under {} but its host pin {} cannot be read; the freeze that wrote the lock left no pin to verify it against",
            ACCEPTANCE_LOCK_FILE,
            task_root.display(),
            path.display()
        )
    })?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("parsing acceptance pin {}", path.display()))
}

/// Exactly the `freeze-acceptance` postcondition: bundle, lock, pin, the PRD's
/// acceptance ids, and the PRD digest the contract froze.
fn verify_acceptance(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    pin: &AcceptancePin,
) -> Result<()> {
    for name in [ACCEPTANCE_CONTRACT_FILE, ACCEPTANCE_LOCK_FILE] {
        verification::read(&task_root.join(name), true)?;
    }
    let prd = verification::read(prd_path, false)
        .with_context(|| format!("reading PRD {}", prd_path.display()))?;
    let text = std::str::from_utf8(&prd).with_context(|| {
        format!(
            "frozen input {} is not UTF-8; restore that input from the accepted publication",
            prd_path.display()
        )
    })?;
    let contract: AcceptanceContract =
        validate_acceptance_bundle(task_root, Some(pin), &acceptance_ids(text)).map_err(
            |error| {
                anyhow::Error::new(error).context(format!(
                    "frozen acceptance contract under {} does not verify",
                    task_root.display()
                ))
            },
        )?;
    let actual = content_digest(&prd);
    if contract.prd.digest != actual {
        return Err(anyhow!(
            "{} under {} was frozen against PRD digest {} but {} now digests to {}; the frozen chain belongs to a different PRD; restore the named input to its frozen digest before retrying verification",
            ACCEPTANCE_CONTRACT_FILE,
            task_root.display(),
            contract.prd.digest,
            prd_path.display(),
            actual
        ));
    }
    // A frozen contract carrying a check the judge did not accept is not a
    // stage to build on: that check can never run. Reusing it skipped the
    // acceptance author and left the refusal to launch time.
    let refuted = super::workflow_task_set::non_accepted_ids(&contract);
    if !refuted.is_empty() {
        return Err(verification::refuted(
            &contract,
            task_root,
            format!(
                "frozen acceptance contract under {} carries check(s) the judge did not accept ({}), which can never run; repair them before decomposing on it: {}",
                task_root.display(),
                refuted.iter().cloned().collect::<Vec<_>>().join(", "),
                super::workflow_task_set::republish::reauthor_command(
                    project_root,
                    task_root,
                    &contract,
                    &refuted
                )
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "workflow_decompose_frozen_chain_fixture.rs"]
pub(crate) mod test_support;
#[cfg(test)]
#[path = "workflow_decompose_frozen_chain_tests.rs"]
mod tests;
