//! Durable authority for a recovery re-freeze of an existing run's chain.
//! Recovery records launch anchors before moving anything; the enforce-gated
//! freeze publishes its completion receipt atomically with the new chain.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::task_set_contract::{
    AcceptancePin, TASK_SKELETON_FILE, TASK_SKELETON_LOCK_FILE, content_digest,
};
use archon_workflow::task_set_lineage::{
    ChainHistory, ChainProof, LaunchLineage, PinTransition, verify_reached_from,
};
use archon_workflow::task_skeleton::{TaskSkeleton, TaskSkeletonLock};
use archon_workflow::{PortableAcceptanceIdentityV1, WorkflowStore};
use serde::{Deserialize, Serialize};

use super::{
    PreparedAcceptanceFreeze, create_dir_all_durably, sync_parent, validate_destination,
    validate_existing_parents, write_durably,
};
const TRIGGER: &str = "recovery-refreeze:";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Recovery {
    transaction: String,
    task_root: PathBuf,
    runs: BTreeMap<String, PortableAcceptanceIdentityV1>,
    prior: Option<AcceptancePin>,
    skeleton: Option<TaskSkeleton>,
    completed: Option<Completion>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Completion {
    identity: PortableAcceptanceIdentityV1,
    lineage: Vec<PinTransition>,
    ids: BTreeSet<String>,
}

#[derive(Debug)]
pub(super) struct Publication {
    pub(super) files: Vec<(PathBuf, Vec<u8>)>,
    pub(super) expected: (PathBuf, Option<String>),
}

pub(crate) fn path(pin: &Path) -> PathBuf {
    pin.with_extension("recovery-lineage")
}

fn read(pin: &Path) -> Result<(Vec<Recovery>, Option<String>)> {
    let path = path(pin);
    if let Some(parent) = path.parent() {
        validate_existing_parents(&path, parent)?;
    }
    match std::fs::read(&path) {
        Ok(bytes) => Ok((
            serde_json::from_slice(&bytes).context("reading recovery lineage")?,
            Some(content_digest(&bytes)),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((Vec::new(), None)),
        Err(error) => Err(error.into()),
    }
}

/// Idempotent, durable, and called before the first move-aside, including
/// retries after a partial move. New runs cannot acquire an earlier event.
pub(crate) fn record_unfreeze(pin: &Path, tasks: &Path, transaction: &str) -> Result<()> {
    let (mut records, _) = read(pin)?;
    if records
        .iter()
        .any(|record| record.transaction == transaction)
    {
        // Retry a directory flush a previous attempt may have failed.
        return sync_parent(&path(pin));
    }
    let task_root = tasks.canonicalize().map(archon_shell::paths::plain)?;
    let archon = pin
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| anyhow!("pin has no store"))?;
    let store = WorkflowStore::new(archon.join("workflows"));
    let mut runs = BTreeMap::new();
    match std::fs::read_dir(store.root()) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let id = entry.file_name().to_string_lossy().into_owned();
                validate_existing_parents(&entry.path(), store.root())?;
                // A run without a usable launch snapshot grants no authority.
                if let Ok(snapshot) = crate::command::acceptance_chain::launch_snapshot(&store, &id)
                    && Path::new(&snapshot.canonical_task_root_identity)
                        .canonicalize()
                        .map(archon_shell::paths::plain)
                        .ok()
                        .as_ref()
                        == Some(&task_root)
                    && let Some(identity) = snapshot.portable_acceptance_identity
                {
                    runs.insert(id, identity);
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let prior = prior_pin(pin, tasks)?;
    let skeleton = prior.as_ref().and_then(|prior| {
        let bytes = std::fs::read(tasks.join(TASK_SKELETON_FILE)).ok()?;
        (prior.skeleton_digest.as_deref() == Some(content_digest(&bytes).as_str()))
            .then(|| serde_json::from_slice::<TaskSkeleton>(&bytes).ok())
            .flatten()
    });
    if prior.is_none() && runs.is_empty() {
        // No launch or retained pin can need adoption of this recovery.
        return Ok(());
    }
    records.push(Recovery {
        transaction: transaction.into(),
        task_root,
        runs,
        prior,
        skeleton,
        completed: None,
    });
    let dest = path(pin);
    let parent = dest
        .parent()
        .ok_or_else(|| anyhow!("recovery lineage has no parent"))?;
    create_dir_all_durably(parent)?;
    let temp = dest.with_extension("recovery-lineage.tmp");
    let scopes = [(parent.to_path_buf(), None)];
    validate_destination(&dest, &scopes)?;
    validate_destination(&temp, &scopes)?;
    write_durably(&temp, &serde_json::to_vec_pretty(&records)?)?;
    std::fs::rename(&temp, &dest)?;
    sync_parent(&dest)
}

/// Import a completed move-aside from a publisher that recorded no recovery
/// lineage. Preserve an already pending event through repeated acquisitions.
pub(crate) fn record_unfrozen_if_missing(
    pin: &Path,
    tasks: &Path,
    transaction: &str,
) -> Result<()> {
    let (records, _) = read(pin)?;
    if records.iter().any(|record| {
        record.completed.is_none() && (record.prior.is_some() || !record.runs.is_empty())
    }) {
        return sync_parent(&path(pin));
    }
    record_unfreeze(pin, tasks, transaction)
}

/// Old recoveries had no pending marker. After their pin move, recover a
/// retained pin only when it authenticates the still-live skeleton (or the
/// acceptance-only contract). Historical inspection files grant no new event.
fn prior_pin(pin: &Path, tasks: &Path) -> Result<Option<AcceptancePin>> {
    let decode = |path: &Path| {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AcceptancePin>(&bytes).ok())
    };
    if let Some(prior) = decode(pin) {
        return Ok(Some(prior));
    }
    let (Some(parent), Some(name)) = (pin.parent(), pin.file_name()) else {
        return Ok(None);
    };
    let prefix = format!(".{}.unverified-", name.to_string_lossy());
    let skeleton = std::fs::read(tasks.join(TASK_SKELETON_FILE))
        .ok()
        .map(|bytes| content_digest(&bytes));
    let contract =
        std::fs::read(tasks.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE))
            .ok()
            .map(|bytes| content_digest(&bytes));
    let root = tasks.canonicalize().map(archon_shell::paths::plain)?;
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(id) = name.strip_prefix(&prefix) else {
            continue;
        };
        if id.len() != 32 || uuid::Uuid::parse_str(id).is_err() || !entry.file_type()?.is_file() {
            continue;
        }
        let Some(prior) = decode(&entry.path()) else {
            continue;
        };
        let bound = match &prior.skeleton_digest {
            Some(digest) => skeleton.as_ref() == Some(digest),
            None => contract.as_ref() == Some(&prior.acceptance_digest),
        };
        if bound
            && Path::new(&prior.task_root)
                .canonicalize()
                .map(archon_shell::paths::plain)
                .ok()
                .as_ref()
                == Some(&root)
        {
            return Ok(Some(prior));
        }
    }
    Ok(None)
}

impl PreparedAcceptanceFreeze {
    /// Only a persisted launch anchor captured by recovery can authorize a
    /// whole-set re-freeze for that run. Ordinary freezes keep their policy.
    pub(crate) fn record_recovery_refreeze(&mut self) -> Result<()> {
        let pin_path = super::acceptance_pin_path(&self.project_root, &self.tasks_root);
        let (mut records, digest) = read(&pin_path)?;
        let Some(record) = records
            .iter_mut()
            .rev()
            .find(|record| record.completed.is_none())
        else {
            return Ok(());
        };
        let Some(from) = record
            .prior
            .as_ref()
            .map(AcceptancePin::identity)
            .or_else(|| record.runs.values().next().cloned())
        else {
            return Ok(());
        };
        if record.task_root
            != self
                .tasks_root
                .canonicalize()
                .map(archon_shell::paths::plain)?
        {
            return Err(anyhow!("recovery task root changed"));
        }
        let mut files = Vec::new();
        if let Some(skeleton) = &record.skeleton {
            let mut skeleton = skeleton.clone();
            skeleton.acceptance_digest = self.pin.acceptance_digest.clone();
            let bytes = serde_json::to_vec_pretty(&skeleton)?;
            let digest = content_digest(&bytes);
            let gate = record
                .prior
                .as_ref()
                .and_then(|pin| pin.skeleton_gate.clone())
                .ok_or_else(|| anyhow!("recovered skeleton has no gate provenance"))?;
            let lock = TaskSkeletonLock {
                algorithm: "blake3".into(),
                digest: digest.clone(),
                acceptance_digest: self.pin.acceptance_digest.clone(),
                gate: gate.clone(),
            };
            self.pin.skeleton_digest = Some(digest);
            self.pin.skeleton_gate = Some(gate);
            files.push((self.tasks_root.join(TASK_SKELETON_FILE), bytes));
            files.push((
                self.tasks_root.join(TASK_SKELETON_LOCK_FILE),
                serde_json::to_vec_pretty(&lock)?,
            ));
        }
        let contract = self.contract()?;
        let ids = contract
            .acceptance
            .into_iter()
            .chain(contract.supplementary)
            .map(|entry| entry.id)
            .collect::<BTreeSet<_>>();
        if let Some(prior) = &record.prior {
            self.pin.fidelity_waivers = prior.fidelity_waivers.clone();
        }
        self.pin.lineage = record
            .prior
            .as_ref()
            .map(|pin| pin.lineage.clone())
            .unwrap_or_default();
        self.pin.lineage.push(PinTransition::extending(
            &self.pin.lineage,
            from,
            self.pin.identity(),
            ids.clone(),
            &format!("{TRIGGER}{}", record.transaction),
        ));
        record.completed = Some(Completion {
            identity: self.pin.identity(),
            lineage: self.pin.lineage.clone(),
            ids,
        });
        files.push((path(&pin_path), serde_json::to_vec_pretty(&records)?));
        self.recovery = Some(Publication {
            files,
            expected: (path(&pin_path), digest),
        });
        Ok(())
    }
}

/// A recovery hop needs BOTH the durable run-bound authorization and the
/// exact recorded lineage. Later per-check hops are still checked normally.
pub(crate) fn verify(
    launch: &PortableAcceptanceIdentityV1,
    pin: &AcceptancePin,
    pin_path: &Path,
    tasks: &Path,
    run: &str,
) -> Result<Option<ChainProof>> {
    if pin.identity() == *launch {
        return Ok(None);
    }
    if let Some(start) = pin.lineage.iter().rposition(|link| link.from == *launch)
        && !pin.lineage[start..]
            .iter()
            .any(|link| link.trigger.starts_with(TRIGGER))
    {
        return Ok(None);
    }
    if !pin
        .lineage
        .iter()
        .any(|link| link.trigger.starts_with(TRIGGER))
    {
        return Ok(None);
    }
    let (records, _) = read(pin_path)?;
    let root = tasks.canonicalize().map(archon_shell::paths::plain)?;
    for record in records.iter().rev() {
        let Some(done) = &record.completed else {
            continue;
        };
        if record.task_root != root
            || record.runs.get(run) != Some(launch)
            || !pin.lineage.starts_with(&done.lineage)
        {
            continue;
        }
        super::publish::verify_recovered_chain(pin_path, tasks)
            .map_err(|reason| anyhow!(reason))?;
        let mut proof = verify_reached_from(
            &done.identity,
            LaunchLineage::Recorded,
            pin,
            tasks,
            &ChainHistory::for_pin(pin_path),
        )?;
        proof.identical = false;
        proof.recorded_hops = proof.recorded_hops.map(|hops| hops + 1);
        proof.changed_ids.extend(done.ids.iter().cloned());
        return Ok(Some(proof));
    }
    Err(anyhow!(
        "chain check unrecorded_change failed: recovery re-freeze has no matching durable launch authorization and lineage for run {run}"
    ))
}
