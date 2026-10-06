//! PLAN-11: the sources a frozen acceptance check runs, pinned by digest, so
//! an implementing agent cannot edit the test that judges it.
//!
//! # Where the pins live, and why not in the contract
//!
//! A frozen task set's pins are a SIDECAR in the engine's pin store,
//! `.archon/task-set-pins/check-sources/<task-root key>.json`, beside the
//! acceptance pin (same key). The pin store is already part of the frozen
//! chain (`residual_paths::frozen_chain_file`): no grant opens it and a
//! landing that touches it is rejected. The sidecar binds the contract by its
//! `acceptance_digest`, and the bytes of every pinned source are filed by
//! digest in a write-once blob store beside it, so a pinned version can be
//! restored and shown to the judge. Putting the pins INTO the contract would
//! change the bytes, and so the digest, of every contract already frozen:
//! every lock, pin, launch identity and replay of an existing task set would
//! stop verifying. The sidecar leaves them byte-identical.
//!
//! # Old contracts
//!
//! A task set frozen before PLAN-11 has no sidecar. A run over it pins its
//! checks' sources itself the first time it needs them (its first landing or
//! acceptance round), from the tree as it is then, into the run's own
//! record `<run>/v2/check-sources/pins.json`; origin
//! [`ORIGIN_RUN_FIRST_USE`]. The next `freeze-acceptance` or republish writes
//! the frozen sidecar, and from then on the run reads that.
//!
//! # What changes a pin
//!
//! Only a freeze (every check pinned fresh), a republish (re-authored checks
//! re-pinned, every other entry carried over byte-identical), or a judged
//! re-author request (`check_source_requests`): each re-pin appends a
//! [`RepinLink`] naming the request, the digests and the prior sidecar's
//! digest, whose bytes are filed in the blob store.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::check_source_resolve::{Found, Roots, SourceRoot, Watch, resolve};
use crate::check_source_rust::item_text;
use crate::task_set_contract::{AcceptanceCheck, AcceptanceContract, TrustedCwd, content_digest};

pub const CHECK_SOURCE_PINS_SCHEMA: u32 = 1;
pub const ORIGIN_FREEZE: &str = "freeze";
pub const ORIGIN_REPUBLISH: &str = "republish";
pub const ORIGIN_RUN_FIRST_USE: &str = "run_first_use";

/// Every frozen check's pinned sources, bound to one contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSourcePins {
    pub schema_version: u32,
    pub acceptance_digest: String,
    pub origin: String,
    pub pinned_at: String,
    pub checks: BTreeMap<String, CheckPins>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repins: Vec<RepinLink>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckPins {
    pub command: String,
    pub cwd: SourceRoot,
    pub sources: Vec<PinnedSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub watches: Vec<Watch>,
    /// What the resolver could not follow, with why; recorded, never dropped.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved: Vec<String>,
}

/// One pinned source: a whole file, or one test function in it (`item`).
/// `digest` is `None` for a source absent when it was pinned.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PinnedSource {
    pub root: SourceRoot,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub digest: Option<String>,
    pub role: String,
}

impl PinnedSource {
    pub fn same_source(&self, root: SourceRoot, path: &str, item: Option<&str>) -> bool {
        self.root == root && self.path == path && self.item.as_deref() == item
    }

    /// `path`, or `path` and the item, for messages.
    pub fn label(&self) -> String {
        match &self.item {
            Some(item) => format!("{} ({item})", self.path),
            None => self.path.clone(),
        }
    }
}

/// One judged re-pin of one source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepinLink {
    pub request_id: String,
    pub check_ids: BTreeSet<String>,
    pub root: SourceRoot,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub reason: String,
    pub at: String,
    /// Digest of the sidecar this link was appended to (filed as a blob).
    pub prior_digest: String,
}

/// The bytes a digest names: the current source, or `None` when it is absent.
pub fn current_bytes(
    roots: &Roots,
    root: SourceRoot,
    path: &str,
    item: Option<&str>,
) -> Option<Vec<u8>> {
    bytes_at(&roots.of(root).join(path), item)
}

/// What a link in place of a source reads as: never the bytes it points at,
/// so swapping a pinned file for a link -- even to identical bytes -- is a
/// change, and a link is never applied as a source.
pub const SYMLINK_MARK: &[u8] = b"\0check-source-symlink:";

/// The bytes at `path` as a pinned source sees them (the item's text for an
/// item; a link reads as [`SYMLINK_MARK`] and its target), `None` when it
/// is absent or unreadable.
pub fn bytes_at(path: &Path, item: Option<&str>) -> Option<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        let target = std::fs::read_link(path).ok()?;
        let mut bytes = SYMLINK_MARK.to_vec();
        bytes.extend_from_slice(target.to_string_lossy().as_bytes());
        return Some(bytes);
    }
    let bytes = std::fs::read(path).ok()?;
    match item {
        None => Some(bytes),
        Some(key) => {
            let text = String::from_utf8(bytes).ok()?;
            item_text(&text, key).map(String::into_bytes)
        }
    }
}

/// The frozen checks' commands: (id, command, cwd), a floor check's typed
/// verifier run from the project root.
pub fn check_commands(contract: &AcceptanceContract) -> Vec<(String, String, SourceRoot)> {
    contract
        .acceptance
        .iter()
        .chain(&contract.supplementary)
        .filter_map(|entry| match &entry.check {
            AcceptanceCheck::Command { command, cwd } => Some((
                entry.id.clone(),
                command.clone(),
                match cwd {
                    TrustedCwd::ProjectRoot => SourceRoot::Project,
                    TrustedCwd::RepoRoot => SourceRoot::Repository,
                },
            )),
            AcceptanceCheck::Floor { contract } => contract
                .typed_verifier_command
                .clone()
                .map(|command| (entry.id.clone(), command, SourceRoot::Project)),
        })
        .collect()
}

/// Resolve and pin one check's sources now, filing each present source's
/// bytes in `blobs`.
pub fn pin_check(command: &str, cwd: SourceRoot, roots: &Roots, blobs: &BlobStore) -> CheckPins {
    let resolution = resolve(command, roots.of(cwd), roots);
    let sources = resolution
        .found
        .iter()
        .map(|found| pin_found(found, roots, blobs))
        .collect();
    CheckPins {
        command: command.to_string(),
        cwd,
        sources,
        watches: resolution.watches,
        unresolved: resolution.unresolved,
    }
}

pub fn pin_found(found: &Found, roots: &Roots, blobs: &BlobStore) -> PinnedSource {
    let bytes = current_bytes(roots, found.root, &found.path, found.item.as_deref());
    PinnedSource {
        root: found.root,
        path: found.path.clone(),
        item: found.item.clone(),
        digest: bytes.map(|bytes| blobs.put(&bytes)),
        role: found.role.clone(),
    }
}

/// Every check of `contract` pinned fresh.
pub fn pin_contract(
    contract: &AcceptanceContract,
    acceptance_digest: &str,
    roots: &Roots,
    origin: &str,
    blobs: &BlobStore,
) -> CheckSourcePins {
    let checks = check_commands(contract)
        .into_iter()
        .map(|(id, command, cwd)| (id, pin_check(&command, cwd, roots, blobs)))
        .collect();
    CheckSourcePins {
        schema_version: CHECK_SOURCE_PINS_SCHEMA,
        acceptance_digest: acceptance_digest.to_string(),
        origin: origin.to_string(),
        pinned_at: chrono::Utc::now().to_rfc3339(),
        checks,
        repins: Vec::new(),
    }
}

/// `prior` re-bound to `contract`: an entry whose check is not in
/// `reauthored` and whose command and cwd are unchanged is carried over as it
/// was (a drifted source is never re-pinned by an unrelated republish); every
/// other check is pinned fresh. The re-pin lineage is kept.
pub fn rebind(
    prior: &CheckSourcePins,
    contract: &AcceptanceContract,
    acceptance_digest: &str,
    roots: &Roots,
    origin: &str,
    blobs: &BlobStore,
    reauthored: &BTreeSet<String>,
) -> CheckSourcePins {
    let checks = check_commands(contract)
        .into_iter()
        .map(|(id, command, cwd)| {
            let kept = prior
                .checks
                .get(&id)
                .filter(|pins| {
                    !reauthored.contains(&id) && pins.command == command && pins.cwd == cwd
                })
                .cloned();
            let pins = kept.unwrap_or_else(|| pin_check(&command, cwd, roots, blobs));
            (id, pins)
        })
        .collect();
    CheckSourcePins {
        schema_version: CHECK_SOURCE_PINS_SCHEMA,
        acceptance_digest: acceptance_digest.to_string(),
        origin: origin.to_string(),
        pinned_at: chrono::Utc::now().to_rfc3339(),
        checks,
        repins: prior.repins.clone(),
    }
}

/// A write-once, digest-named store of source bytes.
#[derive(Debug, Clone)]
pub struct BlobStore {
    dir: PathBuf,
    /// Read-only stores `get` also looks in: a run's record re-bound from a
    /// stale frozen sidecar still names the bytes filed beside that sidecar.
    fallback: Vec<PathBuf>,
}

impl BlobStore {
    pub fn at(dir: PathBuf) -> Self {
        Self {
            dir,
            fallback: Vec::new(),
        }
    }

    pub fn with_fallback(mut self, other: &BlobStore) -> Self {
        self.fallback.push(other.dir.clone());
        self
    }

    /// File `bytes` under their digest (best effort: an unwritable store
    /// still returns the digest; `get` then reports it missing).
    pub fn put(&self, bytes: &[u8]) -> String {
        let digest = content_digest(bytes);
        let path = self.dir.join(&digest);
        if self.get(&digest).is_none() {
            let _ = std::fs::create_dir_all(&self.dir);
            let _ = write_atomically(&path, bytes);
        }
        digest
    }

    /// The bytes filed under `digest`, verified to hash to it.
    pub fn get(&self, digest: &str) -> Option<Vec<u8>> {
        if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        std::iter::once(&self.dir)
            .chain(&self.fallback)
            .filter_map(|dir| std::fs::read(dir.join(digest)).ok())
            .find(|bytes| content_digest(bytes) == digest)
    }
}

#[path = "check_source_store.rs"]
mod store;
pub(crate) use store::write_atomically;
pub use store::{PinStore, Repin, load_for_run};

#[cfg(test)]
pub(crate) mod tests_support {
    use crate::task_set_contract::AcceptanceContract;

    /// A frozen contract of accepted command checks run from the repository.
    pub(crate) fn contract(checks: &[(&str, &str)]) -> AcceptanceContract {
        let acceptance: Vec<_> = checks
            .iter()
            .map(|(id, command)| {
                serde_json::json!({
                    "id": id, "criterion": format!("{id} holds"),
                    "check": {"kind": "command", "command": command, "cwd": "repo_root"},
                    "judgment": {"verdict": "accepted", "counterexample": "none", "reason": "ok", "host_call_id": "judge"}
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "schema_version": 1,
            "prd": {"path": "prd.md", "digest": "d"},
            "gap_policy": {},
            "acceptance": acceptance
        }))
        .unwrap()
    }
}

#[cfg(test)]
#[path = "check_source_pins_tests.rs"]
mod tests;
