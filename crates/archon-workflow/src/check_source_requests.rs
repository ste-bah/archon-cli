//! PLAN-11: the recorded re-author path for a pinned check source.
//!
//! A change to a pinned source never lands silently and is never silently
//! lost. A write branch that changed one has the change HELD out of its
//! landing (the rest lands) and a request recorded here carrying the proposed
//! bytes; a source found changed in the tree at acceptance time (edited
//! outside any landing) is recorded the same way. The acceptance round then
//! settles each request (`check_source_settle`): the judge decides, an
//! accepted change is applied and re-pinned with a [`RepinLink`], a refused
//! one stays out (or, found in the tree, is restored to its pinned bytes).
//!
//! Records are append-only: `<run>/v2/check-source-requests/<id>.json` is
//! written once, its settlement once beside it as `<id>.resolved.json`, and
//! every proposed version is filed by digest under `blobs/`.
//!
//! [`RepinLink`]: crate::check_source_pins::RepinLink

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::check_source_pins::{BlobStore, write_atomically};
use crate::check_source_resolve::SourceRoot;
use crate::task_set_contract::content_digest;

pub const CHECK_SOURCE_REQUESTS_DIR: &str = "v2/check-source-requests";
pub const REQUEST_SCHEMA: u32 = 1;
/// Held out of a write branch's landing.
pub const ORIGIN_LANDING: &str = "landing";
/// Found in the tree at acceptance time, changed outside any landing.
pub const ORIGIN_ACCEPTANCE_DRIFT: &str = "acceptance_drift";
pub const VERDICT_ACCEPTED: &str = "accepted";
pub const VERDICT_REFUTED: &str = "refuted";
/// The tree moved on since the proposal: nothing can be applied as proposed.
pub const VERDICT_STALE: &str = "stale";
/// No check to judge it for, or its branch never landed: dropped from the
/// queue untouched -- nothing refuted, deleted, restored or committed.
pub const VERDICT_ORPHANED: &str = "orphaned";
/// A newer proposal for the same source from the same task replaced it.
pub const VERDICT_SUPERSEDED: &str = "superseded";
/// Gap id prefix for the pinned check-source changes a branch had held: a
/// host-owned gap (`residual_plan::host_environment_gap`), never residual
/// work, and dropped from an agent's envelope.
pub const CHECK_SOURCE_HELD_GAP_PREFIX: &str = "check_source_change_held_";
/// Gap id prefix for a branch refused because the pins could not be read.
pub const CHECK_SOURCE_PINS_UNAVAILABLE_GAP_PREFIX: &str = "check_source_pins_unavailable_";

/// One proposed change to one pinned (or watched) check source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceChangeRequest {
    pub schema_version: u32,
    pub request_id: String,
    pub origin: String,
    pub check_ids: BTreeSet<String>,
    pub root: SourceRoot,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub was_pinned: bool,
    pub pinned_digest: Option<String>,
    /// The proposed source (the item's text for an item); `None` deletes it.
    pub proposed_digest: Option<String>,
    /// For an item: the whole proposed file, and the file as it landed with
    /// the item held out -- what the proposal is applied over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_file_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landed_file_digest: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub call_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub branch_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_ids: Vec<String>,
    pub created_at: String,
}

impl SourceChangeRequest {
    pub fn label(&self) -> String {
        match &self.item {
            Some(item) => format!("{} ({item})", self.path),
            None => self.path.clone(),
        }
    }
}

/// How one request was settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestResolution {
    pub request_id: String,
    pub verdict: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub counterexample: String,
    /// Whether the settlement changed the tree: an accepted landing change
    /// applied, or a refused drift restored.
    pub applied: bool,
    pub repinned: bool,
    pub at: String,
}

/// What a new request carries beyond its identity.
pub struct NewRequest<'a> {
    pub origin: &'a str,
    pub check_ids: BTreeSet<String>,
    pub root: SourceRoot,
    pub path: &'a str,
    pub item: Option<&'a str>,
    pub was_pinned: bool,
    pub pinned_digest: Option<String>,
    pub proposed: Option<&'a [u8]>,
    pub proposed_file: Option<&'a [u8]>,
    pub landed_file_digest: Option<String>,
    pub call_id: &'a str,
    pub branch_id: &'a str,
    pub task_ids: Vec<String>,
}

pub fn requests_dir(run_root: &Path) -> PathBuf {
    run_root.join(CHECK_SOURCE_REQUESTS_DIR)
}

pub fn blobs(run_root: &Path) -> BlobStore {
    BlobStore::at(requests_dir(run_root).join("blobs"))
}

/// Record `new`, write-once: the same proposal from the same origin and
/// branch is the same request (a resumed branch records nothing twice).
pub fn record(run_root: &Path, new: NewRequest<'_>) -> Result<SourceChangeRequest, String> {
    let store = blobs(run_root);
    // The proposal is only recorded once its bytes are: a request naming
    // bytes nobody kept could never be judged.
    let filed = |bytes: &[u8]| -> Result<String, String> {
        let digest = store.put(bytes);
        store
            .get(&digest)
            .map(|_| digest)
            .ok_or_else(|| "the proposed bytes could not be filed in the request store".to_string())
    };
    let proposed_digest = new.proposed.map(filed).transpose()?;
    let proposed_file_digest = new.proposed_file.map(filed).transpose()?;
    let identity = serde_json::json!([
        new.origin,
        new.call_id,
        new.branch_id,
        new.root,
        new.path,
        new.item,
        new.pinned_digest,
        proposed_digest,
        proposed_file_digest,
    ]);
    // The same proposal again is the same request while it is pending (a
    // resumed branch records nothing twice); once that request is settled,
    // the proposal reappearing -- a refused change made again -- opens a
    // new generation, so it is judged again rather than lost.
    let mut generation = 0u32;
    let (request_id, path) = loop {
        let keyed = if generation == 0 {
            identity.to_string()
        } else {
            format!("{identity}#{generation}")
        };
        let request_id = format!("csr-{}", &content_digest(keyed.as_bytes())[..16]);
        let path = requests_dir(run_root).join(format!("{request_id}.json"));
        match std::fs::read(&path) {
            Ok(bytes) => {
                let existing: SourceChangeRequest = serde_json::from_slice(&bytes)
                    .map_err(|error| format!("{} is unreadable: {error}", path.display()))?;
                let settled = requests_dir(run_root).join(format!("{request_id}.resolved.json"));
                if !settled.exists() {
                    return Ok(existing);
                }
                generation += 1;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break (request_id, path),
            Err(error) => return Err(format!("{} could not be read: {error}", path.display())),
        }
    };
    let request = SourceChangeRequest {
        schema_version: REQUEST_SCHEMA,
        request_id,
        origin: new.origin.to_string(),
        check_ids: new.check_ids,
        root: new.root,
        path: new.path.to_string(),
        item: new.item.map(str::to_string),
        was_pinned: new.was_pinned,
        pinned_digest: new.pinned_digest,
        proposed_digest,
        proposed_file_digest,
        landed_file_digest: new.landed_file_digest,
        call_id: new.call_id.to_string(),
        branch_id: new.branch_id.to_string(),
        task_ids: new.task_ids,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    std::fs::create_dir_all(requests_dir(run_root)).map_err(|error| error.to_string())?;
    write_atomically(
        &path,
        &serde_json::to_vec_pretty(&request).expect("a request serializes"),
    )
    .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
    supersede_older(run_root, &request)?;
    Ok(request)
}

/// Settle every older pending request for the same source from the same
/// task as superseded by `newer`.
fn supersede_older(run_root: &Path, newer: &SourceChangeRequest) -> Result<(), String> {
    if newer.origin != ORIGIN_LANDING {
        return Ok(());
    }
    for older in pending(run_root)? {
        let same = older.request_id != newer.request_id
            && older.origin == ORIGIN_LANDING
            && older.root == newer.root
            && older.path == newer.path
            && older.item == newer.item
            && older.task_ids == newer.task_ids;
        if same {
            settle_record(
                run_root,
                &RequestResolution {
                    request_id: older.request_id.clone(),
                    verdict: VERDICT_SUPERSEDED.into(),
                    reason: format!("superseded by request {}", newer.request_id),
                    counterexample: String::new(),
                    applied: false,
                    repinned: false,
                    at: chrono::Utc::now().to_rfc3339(),
                },
            )?;
        }
    }
    Ok(())
}

/// Every recorded request with its settlement, if any, oldest first. A
/// record that cannot be read is an error, never skipped.
pub fn all(
    run_root: &Path,
) -> Result<Vec<(SourceChangeRequest, Option<RequestResolution>)>, String> {
    let dir = requests_dir(run_root);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{} could not be listed: {error}", dir.display())),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        if id.ends_with(".resolved") || id.starts_with('.') || !entry.path().is_file() {
            continue;
        }
        let unreadable = |error: String| {
            format!(
                "request record {} is unreadable: {error}",
                entry.path().display()
            )
        };
        let bytes = std::fs::read(entry.path()).map_err(|e| unreadable(e.to_string()))?;
        let request: SourceChangeRequest =
            serde_json::from_slice(&bytes).map_err(|e| unreadable(e.to_string()))?;
        let settled = dir.join(format!("{id}.resolved.json"));
        let resolution = match std::fs::read(&settled) {
            Ok(bytes) => {
                Some(serde_json::from_slice(&bytes).map_err(|e| unreadable(e.to_string()))?)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(unreadable(error.to_string())),
        };
        out.push((request, resolution));
    }
    out.sort_by(|a: &(SourceChangeRequest, Option<RequestResolution>), b| {
        (a.0.created_at.as_str(), a.0.request_id.as_str())
            .cmp(&(b.0.created_at.as_str(), b.0.request_id.as_str()))
    });
    Ok(out)
}

/// The requests not settled yet, oldest first.
pub fn pending(run_root: &Path) -> Result<Vec<SourceChangeRequest>, String> {
    Ok(all(run_root)?
        .into_iter()
        .filter(|(_, resolution)| resolution.is_none())
        .map(|(request, _)| request)
        .collect())
}

/// Record how `request` was settled. Write-once.
pub fn settle_record(run_root: &Path, resolution: &RequestResolution) -> Result<(), String> {
    let path = requests_dir(run_root).join(format!("{}.resolved.json", resolution.request_id));
    if path.exists() {
        return Ok(());
    }
    write_atomically(
        &path,
        &serde_json::to_vec_pretty(resolution).expect("a resolution serializes"),
    )
    .map_err(|error| format!("{} could not be written: {error}", path.display()))
}

/// The latest refusal of a proposal for `path`, for the next proposer.
pub fn last_refusal(run_root: &Path, path: &str) -> Option<String> {
    all(run_root)
        .ok()?
        .into_iter()
        .rev()
        .find_map(|(request, resolution)| {
            let resolution = resolution?;
            (request.path == path && resolution.verdict == VERDICT_REFUTED).then(|| {
                format!(
                    "request {} was refused by the judge: {}",
                    request.request_id, resolution.reason
                )
            })
        })
}

#[cfg(test)]
#[path = "check_source_requests_tests.rs"]
mod tests;
