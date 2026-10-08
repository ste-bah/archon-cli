//! Ownership of an in-flight ingest of one content hash.
//!
//! An ingest registers its document first and finishes it later. A store pause
//! (`StoreBusy`) or a crash in between leaves the registration `Discovered` or
//! `Ingesting`. Without an owner, the next run would skip that document as a
//! duplicate of itself for ever. So every ingest owns a claim on its content
//! hash from registration until the final status: a live ingest holds it, and
//! an interrupted one leaves it free. A later run resumes a registration only
//! when it can take that free claim.
//!
//! A file-backed store keeps one claim file per content hash beside the
//! database. Every claim file operation (open, take, remove) runs under the
//! store's write lock, the same lock as the hash reservation, so no run can
//! open a claim file that another run is removing. An in-memory store is
//! reachable only from this process, so its claims live in a process table.
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use anyhow::{Result, anyhow};
use cozo::DbInstance;

use crate::models::DocumentStatus;

const CLAIM_CONTEXT: &str = "claim document ingest";

/// Where the claim for one content hash lives. Opened only under the lock.
pub struct ClaimSlot {
    place: Place,
}

enum Place {
    File {
        path: PathBuf,
        file: Option<archon_cozo::OwnerLockFile>,
    },
    Memory(String),
}

/// Ownership of one in-flight ingest. Dropping it (a pause, an error, a
/// panic) releases the claim so a later run can resume the document.
#[derive(Debug)]
pub struct IngestClaim<'a> {
    held: Held<'a>,
}

#[derive(Debug)]
enum Held<'a> {
    File(archon_cozo::OwnerLock<'a>),
    Memory(MemoryClaim),
}

#[derive(Debug)]
struct MemoryClaim(String);

impl Drop for MemoryClaim {
    fn drop(&mut self) {
        memory_claims()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&self.0);
    }
}

fn memory_claims() -> &'static Mutex<HashSet<String>> {
    static CLAIMS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    CLAIMS.get_or_init(|| Mutex::new(HashSet::new()))
}

impl ClaimSlot {
    pub fn for_content(db: &DbInstance, content_hash: &str) -> Result<Self> {
        if content_hash.is_empty() || !content_hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(anyhow!(
                "{CLAIM_CONTEXT}: content hash {content_hash:?} is not a hex digest"
            ));
        }
        let config = archon_cozo::bound_guard_config(db, CLAIM_CONTEXT)?;
        let place = match config.write_lock_path {
            Some(lock) => {
                let name = lock.file_name().and_then(|n| n.to_str()).unwrap_or("cozo");
                let store = name.strip_suffix(".archon-cozo-write.lock").unwrap_or(name);
                let path = lock
                    .with_file_name(format!("{store}.archon-ingest-claims"))
                    .join(format!("{content_hash}.lock"));
                Place::File { path, file: None }
            }
            None => {
                let identity = archon_cozo::in_memory_database_identity(db).ok_or_else(|| {
                    anyhow!("{CLAIM_CONTEXT}: store has neither a lock path nor an identity")
                })?;
                Place::Memory(format!("{identity}:{content_hash}"))
            }
        };
        Ok(Self { place })
    }

    /// Take the claim, or `None` when a live ingest owns it. Call only under
    /// the store's write lock (see the module documentation).
    pub(super) fn try_claim(&mut self) -> Result<Option<IngestClaim<'_>>> {
        match &mut self.place {
            Place::File { path, file } => {
                if file.is_none() {
                    *file = Some(archon_cozo::OwnerLockFile::open(path.clone())?);
                }
                let file = file.as_mut().expect("the claim file was just opened");
                Ok(file.try_own()?.map(|lock| IngestClaim {
                    held: Held::File(lock),
                }))
            }
            Place::Memory(key) => {
                let mut claims = memory_claims()
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                Ok(claims.insert(key.clone()).then(|| IngestClaim {
                    held: Held::Memory(MemoryClaim(key.clone())),
                }))
            }
        }
    }
}

impl IngestClaim<'_> {
    fn complete(self) {
        match self.held {
            Held::File(lock) => lock.complete(),
            Held::Memory(claim) => drop(claim),
        }
    }
}

/// Only the ingest flow's own in-progress states can be resumed. A finished,
/// failed or otherwise owned status is never taken over.
pub(super) fn is_resumable(status: &DocumentStatus) -> bool {
    matches!(
        status,
        DocumentStatus::Discovered | DocumentStatus::Ingesting
    )
}

/// Record the final status of a claimed ingest and end its claim, under one
/// hold of the store's write lock. A later run that finds the final status
/// never needs the claim, and none can open the claim file while it goes.
pub fn finish_claimed_ingest(
    db: &DbInstance,
    document_id: &str,
    status: &DocumentStatus,
    claim: IngestClaim<'_>,
) -> Result<()> {
    super::documents::with_reservation_lock_held(db, || {
        super::update_doc_status(db, document_id, status)?;
        claim.complete();
        Ok(())
    })
}

/// Take the claim for a document that is about to be processed again in
/// place. A live ingest of the same content is an error, not a takeover.
pub fn claim_for_reprocess<'a>(
    db: &DbInstance,
    document_id: &str,
    slot: &'a mut ClaimSlot,
) -> Result<IngestClaim<'a>> {
    super::documents::with_reservation_lock_held(db, || {
        slot.try_claim()?.ok_or_else(|| {
            anyhow!("document {document_id} is being ingested by a live run; reprocess it later")
        })
    })
}
