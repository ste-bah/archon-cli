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
    TASK_SKELETON_LOCK_FILE, content_digest, validate_acceptance_bundle,
};
use archon_workflow::task_skeleton::validate_full_chain;

/// The frozen chain the launcher found and verified under a task root.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct FrozenChainSnapshot {
    pub(crate) acceptance: bool,
    pub(crate) skeleton: bool,
    pub(crate) subjects: Vec<HostCommandSubject>,
    /// Frozen file names whose body file exists under the task root.
    pub(crate) bodies: Vec<String>,
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
    let mut snapshot = FrozenChainSnapshot::default();
    if !task_root.is_dir() {
        return Ok(snapshot);
    }
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
    verify_acceptance(prd_path, task_root, &pin)?;
    snapshot.acceptance = true;
    if !skeleton_lock.exists() {
        return Ok(snapshot);
    }
    let skeleton = validate_full_chain(task_root, &pin).map_err(|error| {
        anyhow!(
            "frozen task skeleton under {} does not verify: {error}",
            task_root.display()
        )
    })?;
    snapshot.skeleton = true;
    for task in &skeleton.tasks {
        let path = task_root.join(&task.file_name);
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_file()) {
            snapshot.bodies.push(task.file_name.clone());
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
pub(crate) fn verify_frozen_stage(
    project_root: &Path,
    prd_path: &Path,
    task_root: &Path,
    stage: FrozenStage,
) -> Result<FrozenChainSnapshot> {
    let snapshot = frozen_chain_snapshot(project_root, prd_path, task_root)?;
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
fn verify_acceptance(prd_path: &Path, task_root: &Path, pin: &AcceptancePin) -> Result<()> {
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
    config: &archon_core::config::ArchonConfig,
) -> Result<()> {
    if config.workflow.gate_mode == archon_core::config::GateMode::Off {
        return Err(anyhow!(
            "gate_mode=off must return before staged frozen-chain verification"
        ));
    }
    let manifest = stage_verify(cwd, stage, tasks, prd, gate_envelope, call_id)?;
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
    let evaluation = match verify_frozen_stage(cwd, &absolute(prd), &absolute(tasks), stage) {
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
        Err(error) => super::workflow_gate::GateEvaluation::new(
            format!("{} failed", stage.command_id()),
            Vec::new(),
        )
        .with_operational_error(format!("{error:#}")),
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
