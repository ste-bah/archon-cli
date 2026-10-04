//! Durable legacy decisions. The old plain-id marker is read as verification
//! intent; all new decisions are atomically saved before any rename/unlink.
use super::journal::{is_transaction_id, rename, sync_parent, write_durably};
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Serialize, Deserialize)]
pub(super) struct Marker {
    pub(super) transaction: String,
    pub(super) decisions: BTreeMap<String, Decision>,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Decision {
    Forward,
    Rollback,
    Discard,
}
impl Marker {
    pub(super) fn load(path: &Path) -> Result<Option<Self>> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let marker = if let Ok(id) = std::str::from_utf8(&bytes)
            && is_transaction_id(id)
        {
            Self::new(id)
        } else {
            serde_json::from_slice(&bytes)?
        };
        if !is_transaction_id(&marker.transaction)
            || marker.decisions.keys().any(|id| !is_transaction_id(id))
        {
            return Err(anyhow!("invalid legacy verification marker"));
        }
        Ok(Some(marker))
    }
    pub(super) fn new(transaction: &str) -> Self {
        Self {
            transaction: transaction.into(),
            decisions: BTreeMap::new(),
        }
    }
    pub(super) fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow!("verification marker has no parent"))?;
        let temp = path.with_extension("publish-verification.tmp");
        let scopes = [(parent.to_path_buf(), None)];
        super::scope::validate_destination(path, &scopes)?;
        super::scope::validate_destination(&temp, &scopes)?;
        write_durably(&temp, &serde_json::to_vec(self)?)?;
        rename(&temp, path)?;
        sync_parent(path)
    }
}
