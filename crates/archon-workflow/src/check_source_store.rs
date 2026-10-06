//! Where a set of check-source pins is kept, and how it is read and
//! replaced (PLAN-11, [`super`]).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{BlobStore, CheckSourcePins, ORIGIN_RUN_FIRST_USE, pin_contract, rebind};
use crate::check_source_resolve::Roots;
use crate::task_set_contract::{
    ACCEPTANCE_CONTRACT_FILE, AcceptanceContract, AcceptancePin, content_digest,
};
use crate::task_set_publish_lock::{PublishLockError, PublishLockFile};

/// Where one set of pins is kept.
#[derive(Debug, Clone)]
pub struct PinStore {
    pub sidecar: PathBuf,
    pub blobs: BlobStore,
    /// The frozen sidecar in the pin store, or a run's own record.
    pub frozen: bool,
    /// The acceptance pin that records the frozen sidecar's digest.
    pub pin: Option<PathBuf>,
    /// The frozen task set's root, whose interrupted publish a repin settles
    /// before it writes (Issue 336).
    pub tasks_root: Option<PathBuf>,
}

impl PinStore {
    /// The frozen task set's sidecar: keyed like its acceptance pin.
    pub fn frozen(project_root: &Path, tasks_root: &Path) -> Self {
        let canonical = tasks_root
            .canonicalize()
            .map(archon_shell::paths::plain)
            .unwrap_or_else(|_| tasks_root.to_path_buf());
        let key = content_digest(canonical.to_string_lossy().as_bytes());
        let dir = crate::task_set_lineage::pin_store_dir(project_root).join("check-sources");
        Self {
            sidecar: dir.join(format!("{key}.json")),
            blobs: BlobStore::at(dir.join("blobs")),
            frozen: true,
            pin: dir.parent().map(|pins| pins.join(format!("{key}.json"))),
            tasks_root: Some(tasks_root.to_path_buf()),
        }
    }

    /// A run's own record, for a contract frozen without a sidecar.
    pub fn for_run(run_root: &Path) -> Self {
        let dir = run_root.join("v2").join("check-sources");
        Self {
            sidecar: dir.join("pins.json"),
            blobs: BlobStore::at(dir.join("blobs")),
            frozen: false,
            pin: None,
            tasks_root: None,
        }
    }

    pub fn read(&self) -> Result<Option<CheckSourcePins>, String> {
        match std::fs::read(&self.sidecar) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| format!("{} is malformed: {error}", self.sidecar.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!(
                "{} could not be read: {error}",
                self.sidecar.display()
            )),
        }
    }

    pub fn bytes(pins: &CheckSourcePins) -> Vec<u8> {
        serde_json::to_vec_pretty(pins).expect("check-source pins serialize")
    }

    /// Replace the sidecar, filing the version it replaces. A frozen
    /// sidecar is replaced under the task set's chain lock and recorded in
    /// its acceptance pin: the new bytes are filed by digest first, the pin
    /// then names that digest (the commit point), and the sidecar follows --
    /// so a crash between the two leaves a pin whose sidecar
    /// [`Self::verified_read`] reads from the filed bytes.
    pub fn write(&self, pins: &CheckSourcePins) -> Result<(), PublishLockError> {
        self.replace(None, pins).map(drop)
    }

    /// [`Self::write`] over the pins `prior` only (Issue 338): the pins the
    /// store binds are read again under the exclusive lock, once a publish a
    /// crash interrupted is settled, and when they are no longer `prior` --
    /// that settlement rolled a new set forward, or another repin ran --
    /// nothing is written and [`Repin::Stale`] carries what is bound now. A
    /// store that binds nothing yet is written.
    pub fn write_over(
        &self,
        prior: &CheckSourcePins,
        pins: &CheckSourcePins,
    ) -> Result<Repin, PublishLockError> {
        self.replace(Some(prior), pins)
    }

    fn replace(
        &self,
        prior: Option<&CheckSourcePins>,
        pins: &CheckSourcePins,
    ) -> Result<Repin, PublishLockError> {
        let bytes = Self::bytes(pins);
        if let Ok(prior) = std::fs::read(&self.sidecar) {
            self.blobs.put(&prior);
        }
        if let Some(parent) = self.sidecar.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let _lock = match (&self.pin, self.frozen) {
            (Some(pin), true) => Some(chain_lock(pin)?),
            _ => None,
        };
        // The pin and the sidecar change together under the publish lock the
        // host's publishes and consistent readers hold (Issue 294), held
        // exclusive, and only once a publish a crash interrupted is settled:
        // never written over a half-applied set (Issue 336).
        let _publish = match (&self.pin, self.frozen) {
            (Some(pin), true) => PublishLockFile::hold(pin, self.tasks_root.as_deref())?,
            _ => None,
        };
        // A store that binds nothing yet holds nothing to overwrite.
        if let Some(prior) = prior
            && let Some(now) = self.verified_read()?
            && Self::bytes(&now) != Self::bytes(prior)
        {
            return Ok(Repin::Stale(now));
        }
        if let (Some(pin_path), true) = (&self.pin, self.frozen)
            && pin_path.is_file()
        {
            let digest = self.blobs.put(&bytes);
            if self.blobs.get(&digest).is_none() {
                return Err(format!(
                    "the new pins could not be filed under {}",
                    self.sidecar.display()
                )
                .into());
            }
            let mut pin = read_pin(pin_path)?;
            pin.check_sources_digest = Some(digest);
            write_atomically(
                pin_path,
                &serde_json::to_vec_pretty(&pin).expect("a pin serializes"),
            )
            .map_err(|error| format!("{} could not be written: {error}", pin_path.display()))?;
        }
        write_atomically(&self.sidecar, &bytes)
            .map_err(|error| format!("{} could not be written: {error}", self.sidecar.display()))?;
        Ok(Repin::Written)
    }

    /// [`Self::read`], held to the digest the acceptance pin records when
    /// it records one: a sidecar that does not hash to it is read from the
    /// bytes filed under it, else refused. A reader never writes (Issue
    /// 338): the sidecar itself is put back by [`Self::restore_sidecar`],
    /// under the exclusive lock, or by the next repin.
    pub fn verified_read(&self) -> Result<Option<CheckSourcePins>, String> {
        match self.unbound_sidecar()? {
            None => self.read(),
            Some((_, filed)) => serde_json::from_slice(&filed)
                .map(Some)
                .map_err(|error| format!("{} is malformed: {error}", self.sidecar.display())),
        }
    }

    /// The sidecar's bytes as the pin binds them, when the sidecar on disk
    /// does not hash to the pin's digest: its pin path and the filed copy.
    /// Refused when no copy is filed.
    fn unbound_sidecar(&self) -> Result<Option<(&Path, Vec<u8>)>, String> {
        let Some(pin_path) = self.pin.as_deref().filter(|path| path.is_file()) else {
            return Ok(None);
        };
        let Some(digest) = read_pin(pin_path)?.check_sources_digest else {
            return Ok(None);
        };
        let actual = std::fs::read(&self.sidecar)
            .ok()
            .map(|bytes| content_digest(&bytes));
        if actual.as_deref() == Some(digest.as_str()) {
            return Ok(None);
        }
        match self.blobs.get(&digest) {
            Some(filed) => Ok(Some((pin_path, filed))),
            None => Err(format!(
                "{} does not hash to the digest {digest} its acceptance pin {} records, and no copy is filed under it; re-run `workflow freeze-acceptance`",
                self.sidecar.display(),
                pin_path.display()
            )),
        }
    }

    /// Put a frozen sidecar a crashed repin left behind its pin back to the
    /// bytes the pin binds, under the exclusive publish lock (any journal
    /// settled first), so no reader writes. Nothing is done when it already
    /// binds, when no copy is filed (the read reports it), or when this
    /// thread holds the lock (its holder's read takes the filed copy).
    pub fn restore_sidecar(&self) -> Result<(), PublishLockError> {
        let Some(pin) = self.pin.as_deref().filter(|_| self.frozen) else {
            return Ok(());
        };
        if crate::task_set_publish_lock::held_here(&crate::task_set_publish_lock::lock_path(pin))
            || !matches!(self.unbound_sidecar(), Ok(Some(_)))
        {
            return Ok(());
        }
        let _write = PublishLockFile::hold_journal_settled(pin, self.tasks_root.as_deref())?;
        if let Some((_, filed)) = self.unbound_sidecar()? {
            write_atomically(&self.sidecar, &filed).map_err(|error| {
                format!("{} could not be restored: {error}", self.sidecar.display())
            })?;
        }
        Ok(())
    }
}

/// What [`PinStore::write_over`] did.
#[derive(Debug, Clone, PartialEq)]
pub enum Repin {
    Written,
    /// The store binds these pins, not the caller's prior: nothing was
    /// written.
    Stale(CheckSourcePins),
}

fn read_pin(path: &Path) -> Result<AcceptancePin, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is malformed: {error}", path.display()))
}

/// The task set's chain lock held by [`chain_lock`]. Dropping it unlocks the
/// file at once (Issue 330): a close alone leaves the lock held while a child
/// that any thread forked still shares the open file before its `exec`, and a
/// freeze or repair that tries the lock then is refused although it is free.
pub struct ChainLockGuard(std::fs::File);

impl Drop for ChainLockGuard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

/// The task set's chain lock, the one every freeze and republish of it
/// holds (`<pin>.chain.lock`), waited for up to a minute.
pub fn chain_lock(pin_path: &Path) -> Result<ChainLockGuard, String> {
    let path = pin_path.with_extension("chain.lock");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|error| format!("opening chain lock {}: {error}", path.display()))?;
    for _ in 0..120 {
        if file.try_lock().is_ok() {
            return Ok(ChainLockGuard(file));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(format!(
        "another freeze or repair of this task set holds {}",
        path.display()
    ))
}

pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// The pins a run reads for the contract under `tasks_root`: the frozen
/// sidecar when it binds the contract; else the run's own record when it
/// does; else the run pins now (re-bound from whichever stale set exists, or
/// fresh) into its own record. `None` when the task root holds no readable
/// contract: there is nothing to pin. A publish of the set a crash
/// interrupted is settled first, under the exclusive lock (Issue 338); one
/// no settlement can settle is [`PublishLockError::Unsettled`], on which the
/// caller's run pauses.
pub fn load_for_run(
    run_root: &Path,
    project_root: &Path,
    tasks_root: &Path,
    roots: &Roots,
) -> Result<Option<(PinStore, CheckSourcePins)>, PublishLockError> {
    let frozen = PinStore::frozen(project_root, tasks_root);
    // Issue 294: the contract, the pin and the sidecar are read as one
    // version, never mid-republish (the publish lock every publisher holds),
    // held shared beside other readers (Issue 336), and never over a journal
    // a crashed publish left (Issue 338).
    frozen.restore_sidecar()?;
    let _read = match &frozen.pin {
        Some(pin) => PublishLockFile::hold_shared_settled(pin, tasks_root)?,
        None => None,
    };
    let path = tasks_root.join(ACCEPTANCE_CONTRACT_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // Nothing to pin only if nothing was ever frozen here: a lock or
            // a pinned sidecar without its contract is a broken chain.
            let lock = tasks_root.join(crate::task_set_contract::ACCEPTANCE_LOCK_FILE);
            if lock.exists() || frozen.sidecar.exists() {
                return Err(format!(
                    "{} is missing although the task set was frozen",
                    path.display()
                )
                .into());
            }
            return Ok(None);
        }
        Err(error) => return Err(format!("{} could not be read: {error}", path.display()).into()),
    };
    let contract: AcceptanceContract = serde_json::from_slice(&bytes)
        .map_err(|error| format!("the acceptance contract is malformed: {error}"))?;
    let digest = content_digest(&bytes);
    let mut run = PinStore::for_run(run_root);
    run.blobs = run.blobs.with_fallback(&frozen.blobs);
    let frozen_pins = frozen.verified_read()?;
    if let Some(pins) = frozen_pins
        .as_ref()
        .filter(|p| p.acceptance_digest == digest)
    {
        return Ok(Some((frozen, pins.clone())));
    }
    let run_pins = run.read()?;
    if let Some(pins) = run_pins.as_ref().filter(|p| p.acceptance_digest == digest) {
        return Ok(Some((run, pins.clone())));
    }
    let pins = match run_pins.or(frozen_pins) {
        Some(stale) => rebind(
            &stale,
            &contract,
            &digest,
            roots,
            ORIGIN_RUN_FIRST_USE,
            &run.blobs,
            &BTreeSet::new(),
        ),
        None => pin_contract(&contract, &digest, roots, ORIGIN_RUN_FIRST_USE, &run.blobs),
    };
    run.write(&pins)?;
    Ok(Some((run, pins)))
}

#[cfg(test)]
#[path = "check_source_store_tests.rs"]
mod tests;
