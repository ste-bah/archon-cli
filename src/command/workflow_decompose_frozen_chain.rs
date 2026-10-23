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
    let read = frozen_chain_snapshot_read_only(project_root, prd_path, task_root);
    let deferred = read
        .as_ref()
        .err()
        .is_some_and(|error| format!("{error:#}").contains("publish recovery is pending"));
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
    let snapshot = frozen_chain_snapshot(project_root, prd_path, task_root)?;
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
}

impl FrozenChainSnapshot {
    /// The script argument: camelCase subjects, exactly as `HostCommandSubject`
    /// serialises for a host outcome.
    pub(crate) fn to_argument(&self) -> serde_json::Value {
        serde_json::json!({
            "acceptance": self.acceptance,
            "skeleton": self.skeleton,
            "subjects": self.subjects,
            "bodies": self.bodies,
        })
    }
}

/// The stage a `verify-frozen-chain` call re-verifies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrozenStage {
    Acceptance,
    Skeleton,
}

impl FrozenStage {
    pub(crate) fn parse(text: &str) -> Result<Self> {
        match text {
            "acceptance" => Ok(Self::Acceptance),
            "skeleton" => Ok(Self::Skeleton),
            other => Err(anyhow!(
                "verify-frozen-chain stage must be acceptance or skeleton, found '{other}'"
            )),
        }
    }

    pub(crate) fn command_id(self) -> &'static str {
        match self {
            Self::Acceptance => "verify-frozen-acceptance",
            Self::Skeleton => "verify-frozen-skeleton",
        }
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
    if task_root.is_dir() {
        let pin_path = super::workflow_task_set::acceptance_pin_path(project_root, task_root);
        let lock_path = archon_workflow::task_set_publish_lock::lock_path(&pin_path);
        if !lock_path.exists() {
            let [journal, _] = archon_workflow::task_set_publish_lock::journal_paths(&pin_path);
            if journal.exists() {
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
        snapshot_locked(project_root, prd_path, task_root)
    })
}

/// Read under the chain lock; recovery callers settle interrupted publishes first.
pub(crate) fn with_chain_read<T>(
    project_root: &Path,
    task_root: &Path,
    recover: bool,
    read: impl FnOnce() -> Result<T>,
) -> Result<T> {
    if !task_root.is_dir() {
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
    if journal.exists() {
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
    let mut snapshot = FrozenChainSnapshot::default();
    if !task_root.is_dir() {
        return Ok(snapshot);
    }
    #[cfg(test)]
    super::workflow_task_set::reader_test_step("decomposition-read");
    let acceptance_lock = task_root.join(ACCEPTANCE_LOCK_FILE);
    let skeleton_lock = task_root.join(TASK_SKELETON_LOCK_FILE);
    if !acceptance_lock.exists() {
        if skeleton_lock.exists() {
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
    verify_acceptance(project_root, prd_path, task_root, &pin)?;
    snapshot.acceptance = true;
    if !skeleton_lock.exists() {
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
    let skeleton = validate_full_chain(task_root, &pin).map_err(|error| {
        anyhow!(
            "frozen task skeleton under {} does not verify: {error}",
            task_root.display()
        )
    })?;
    snapshot.skeleton = true;
    for file_name in [TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE] {
        let path = task_root.join(file_name);
        let bytes = std::fs::read(&path)
            .with_context(|| format!("reading frozen task-chain artifact {}", path.display()))?;
        snapshot
            .preserved_digests
            .push((file_name.to_string(), content_digest(&bytes)));
    }
    for task in &skeleton.tasks {
        let path = task_root.join(&task.file_name);
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
            snapshot.bodies.push(task.file_name.clone());
            let bytes = std::fs::read(&path)
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
    Ok(snapshot)
}

/// Verify one frozen stage the way the launcher did, for the
/// `verify-frozen-chain` host command. A stage that is not frozen is an
/// error: the script only asks about a stage the launcher reported frozen.
fn verify_frozen_stage_with_recovery(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    stage: FrozenStage,
    recover: bool,
) -> Result<FrozenChainSnapshot> {
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
    let bytes = std::fs::read(&path).with_context(|| {
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
    let prd =
        std::fs::read(prd_path).with_context(|| format!("reading PRD {}", prd_path.display()))?;
    let text = std::str::from_utf8(&prd).context("PRD is not UTF-8")?;
    let contract: AcceptanceContract =
        validate_acceptance_bundle(task_root, Some(pin), &acceptance_ids(text)).map_err(
            |error| {
                anyhow!(
                    "frozen acceptance contract under {} does not verify: {error}",
                    task_root.display()
                )
            },
        )?;
    let actual = content_digest(&prd);
    if contract.prd.digest != actual {
        return Err(anyhow!(
            "{} under {} was frozen against PRD digest {} but {} now digests to {}; the frozen chain belongs to a different PRD",
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
        return Err(anyhow!(
            "frozen acceptance contract under {} carries check(s) the judge did not accept ({}), which can never run; repair them before decomposing on it: {}",
            task_root.display(),
            refuted.iter().cloned().collect::<Vec<_>>().join(", "),
            super::workflow_task_set::republish::reauthor_command(
                project_root,
                task_root,
                &contract,
                &refuted
            )
        ));
    }
    Ok(())
}

/// The `workflow verify-frozen-chain` trusted child behind the
/// `verify-frozen-acceptance` / `verify-frozen-skeleton` capabilities. It
/// publishes nothing: its gate envelope is the whole staged output, and the
/// parent's postcondition (the freeze command's own) reads the chain again.
/// A stage that does not verify is reported through the envelope as an
/// operational error, never by exiting non-zero, so the script stops with the
/// reason instead of "no committed publication receipt".
pub(crate) fn handle_staged_verify(
    cwd: &Path,
    stage: &str,
    tasks: &Path,
    prd: &Path,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    read_only: bool,
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            "gate_mode=off must return before staged frozen-chain verification"
        ));
    }
    let manifest = stage_verify(cwd, stage, tasks, prd, gate_envelope, call_id, read_only)?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

/// The staged child's work, minus the print: the envelope under the staging
/// root and the prepared manifest naming it.
pub(crate) fn stage_verify(
    cwd: &Path,
    stage: &str,
    tasks: &Path,
    prd: &Path,
    gate_envelope: Option<&Path>,
    call_id: Option<&str>,
    read_only: bool,
) -> Result<archon_workflow::PreparedPublicationV1> {
    let stage = FrozenStage::parse(stage)?;
    let gate_envelope = gate_envelope.ok_or_else(|| {
        anyhow!("trusted staged verify-frozen-chain requires --gate-envelope <PATH>")
    })?;
    let call_id = call_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("trusted staged verify-frozen-chain requires --call-id <ID>"))?;
    let absolute = |path: &Path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        }
    };
    let evaluation = match verify_frozen_stage_with_recovery(
        cwd,
        &absolute(prd),
        &absolute(tasks),
        stage,
        !read_only,
    ) {
        Ok(snapshot) => super::workflow_gate::GateEvaluation::new(
            format!(
                "{} verified: acceptance={}, skeleton={}, {} frozen subject(s), {} body file(s) present",
                stage.command_id(),
                snapshot.acceptance,
                snapshot.skeleton,
                snapshot.subjects.len(),
                snapshot.bodies.len()
            ),
            Vec::new(),
        ),
        Err(error) => {
            // Issue 338: a set no read can settle pauses the run instead.
            super::workflow_host_command_operational::exit_if_unsettled_publish(&error);
            super::workflow_gate::GateEvaluation::new(
                format!("{} failed", stage.command_id()),
                Vec::new(),
            )
            .with_operational_error(format!("{error:#}"))
        }
    };
    let staging_root = gate_envelope
        .parent()
        .ok_or_else(|| anyhow!("staged verify-frozen-chain envelope has no parent"))?;
    super::workflow_gate_envelope::stage_gate_evaluation(
        staging_root,
        gate_envelope,
        call_id,
        stage.command_id(),
        evaluation,
        Vec::new(),
    )
}

#[cfg(test)]
#[path = "workflow_decompose_frozen_chain_fixture.rs"]
pub(crate) mod test_support;
#[cfg(test)]
#[path = "workflow_decompose_frozen_chain_tests.rs"]
mod tests;
