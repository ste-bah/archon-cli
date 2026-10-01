//! PLAN-11: the frozen check-source pins a freeze or republish publishes in
//! the same atomic transaction as the chain it binds
//! (`archon_workflow::check_source_pins`).
//!
//! A whole-set freeze pins every check fresh from the tree. A per-check
//! republish re-pins the re-authored checks (and any whose command moved)
//! and carries every other entry over unchanged, lineage included, so an
//! unrelated repair never blesses a source that drifted. A task set frozen
//! before PLAN-11 has no sidecar yet: its first republish pins it fresh, and
//! until then a run pins it for itself at first use.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use archon_workflow::check_source_pins::{
    ORIGIN_FREEZE, ORIGIN_REPUBLISH, PinStore, pin_contract, rebind,
};
use archon_workflow::check_source_resolve::Roots;
use archon_workflow::task_set_contract::{AcceptanceContract, content_digest};

/// The sidecar (path, bytes) binding `contract_bytes`, for the publish
/// transaction. `reauthored` is `None` for a whole-set freeze, which pins
/// fresh. A republish re-binds the prior pins -- the frozen sidecar, or for
/// a task set frozen before PLAN-11 the newest run record that binds the
/// contract being replaced -- and refuses while any run holds a pending
/// source change for a check it re-authors: that change is settled first.
/// Every read error is the publish's error, never a fresh pin.
pub(crate) fn frozen_sidecar(
    project_root: &Path,
    tasks_root: &Path,
    contract_bytes: &[u8],
    reauthored: Option<&BTreeSet<String>>,
) -> Result<(PathBuf, Vec<u8>)> {
    let contract: AcceptanceContract = serde_json::from_slice(contract_bytes)
        .context("parsing the contract to pin its sources")?;
    let digest = content_digest(contract_bytes);
    let repository = match archon_workflow::repository_record::read_repository_record(tasks_root)
        .map_err(|error| anyhow!("{error}"))
        .context("reading the task set's repository record to pin its checks' sources")?
    {
        Some(record) => {
            let root = PathBuf::from(&record.repository_root);
            if !root.is_dir() {
                return Err(anyhow!(
                    "the task set's repository {} is not a directory, so its checks' sources cannot be pinned",
                    root.display()
                ));
            }
            root
        }
        None => project_root.to_path_buf(),
    };
    let roots = Roots {
        repository: &repository,
        project: project_root,
    };
    let store = PinStore::frozen(project_root, tasks_root);
    let pins = match reauthored {
        None => pin_contract(&contract, &digest, &roots, ORIGIN_FREEZE, &store.blobs),
        Some(ids) => {
            refuse_pending(project_root, ids)?;
            let prior = match store.verified_read().map_err(|error| anyhow!(error))? {
                Some(prior) => Some(prior),
                None => run_record_for(project_root, tasks_root, &store)?,
            };
            match prior {
                Some(prior) => rebind(
                    &prior,
                    &contract,
                    &digest,
                    &roots,
                    ORIGIN_REPUBLISH,
                    &store.blobs,
                    ids,
                ),
                None => pin_contract(&contract, &digest, &roots, ORIGIN_REPUBLISH, &store.blobs),
            }
        }
    };
    if let Ok(prior) = std::fs::read(&store.sidecar) {
        // The version this publish replaces stays readable by digest.
        store.blobs.put(&prior);
    }
    Ok((store.sidecar.clone(), PinStore::bytes(&pins)))
}

/// The runs recorded under the project.
fn runs(project_root: &Path) -> Result<Vec<PathBuf>> {
    let store = archon_workflow::WorkflowStore::project(project_root);
    let dir = store.root();
    match std::fs::read_dir(dir) {
        Ok(entries) => Ok(entries
            .collect::<std::io::Result<Vec<_>>>()
            .with_context(|| format!("listing {}", dir.display()))?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| path.join("v2").is_dir())
            .collect()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("listing {}", dir.display())),
    }
}

fn refuse_pending(project_root: &Path, ids: &BTreeSet<String>) -> Result<()> {
    for run in runs(project_root)? {
        let pending = archon_workflow::check_source_requests::pending(&run)
            .map_err(|error| anyhow!(error))?;
        if let Some(request) = pending
            .iter()
            .find(|r| r.check_ids.iter().any(|id| ids.contains(id)))
        {
            return Err(anyhow!(
                "run {} holds pending source change {} for check(s) {}; it is settled by that run's next acceptance round before the check is re-authored",
                run.display(),
                request.request_id,
                request
                    .check_ids
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    Ok(())
}

/// The newest run record binding the contract this publish replaces, its
/// filed bytes copied beside the frozen sidecar so the pins it carries stay
/// restorable.
fn run_record_for(
    project_root: &Path,
    tasks_root: &Path,
    store: &PinStore,
) -> Result<Option<archon_workflow::check_source_pins::CheckSourcePins>> {
    let path = tasks_root.join(archon_workflow::task_set_contract::ACCEPTANCE_CONTRACT_FILE);
    let prior = match std::fs::read(&path) {
        Ok(bytes) => content_digest(&bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let mut best: Option<(std::time::SystemTime, PinStore, _)> = None;
    for run in runs(project_root)? {
        let record = PinStore::for_run(&run);
        let Some(pins) = record.read().map_err(|error| anyhow!(error))? else {
            continue;
        };
        if pins.acceptance_digest != prior {
            continue;
        }
        let modified = std::fs::metadata(&record.sidecar)
            .and_then(|meta| meta.modified())
            .with_context(|| format!("reading {}", record.sidecar.display()))?;
        if best.as_ref().is_none_or(|(at, _, _)| modified > *at) {
            best = Some((modified, record, pins));
        }
    }
    let Some((_, record, pins)) = best else {
        return Ok(None);
    };
    for source in pins.checks.values().flat_map(|check| &check.sources) {
        if let Some(digest) = &source.digest
            && store.blobs.get(digest).is_none()
            && let Some(bytes) = record.blobs.get(digest)
        {
            store.blobs.put(&bytes);
        }
    }
    Ok(Some(pins))
}

#[cfg(test)]
#[path = "workflow_task_set_check_sources_tests.rs"]
mod tests;
