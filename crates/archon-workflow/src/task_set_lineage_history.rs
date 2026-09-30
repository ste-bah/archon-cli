//! The write-once, digest-named store of chain versions a task set's pin
//! replaced, and the digests an import may file into it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{AcceptancePin, ChainCheck, ChainRefusal, refuse};
use crate::task_set_contract::content_digest;
use crate::v2::PortableAcceptanceIdentityV1;

/// The write-once store of chain versions a task set's pin replaced:
/// `<pin dir>/history/<pin file stem>/<blake3>.json`, outside the task root.
#[derive(Debug, Clone)]
pub struct ChainHistory {
    dir: PathBuf,
}

impl ChainHistory {
    pub fn for_pin(pin_path: &Path) -> Self {
        let stem = pin_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let parent = pin_path.parent().unwrap_or_else(|| Path::new("."));
        Self {
            dir: parent.join("history").join(stem),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self, digest: &str) -> PathBuf {
        self.dir.join(format!("{digest}.json"))
    }

    /// The stored bytes filed under `digest`, verified to hash to it.
    pub fn get(&self, digest: &str) -> Result<Option<Vec<u8>>, ChainRefusal> {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return refuse(
                ChainCheck::HistoryUnavailable,
                format!("{digest:?} is not a blake3 digest, so nothing is filed under it"),
            );
        }
        let path = self.path(digest);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return refuse(
                    ChainCheck::HistoryUnavailable,
                    format!("{} could not be read: {error}", path.display()),
                );
            }
        };
        let actual = content_digest(&bytes);
        if actual != digest {
            return refuse(
                ChainCheck::PreimageCorrupt,
                format!("{} hashes to {actual}, not {digest}", path.display()),
            );
        }
        Ok(Some(bytes))
    }

    /// File `bytes` under their own digest. Write-once: a file already filed
    /// there that hashes to its name is kept. Returns the digest and whether
    /// this call stored it.
    pub fn put(&self, bytes: &[u8]) -> Result<(String, bool), ChainRefusal> {
        let digest = content_digest(bytes);
        // A file filed here that no longer hashes to its name is replaced by
        // these bytes, which do.
        match self.get(&digest) {
            Ok(Some(_)) => return Ok((digest, false)),
            Ok(None) => {}
            Err(refusal) if refusal.check == ChainCheck::PreimageCorrupt => {}
            Err(refusal) => return Err(refusal),
        }
        let unavailable = |error: std::io::Error, path: &Path| ChainRefusal {
            check: ChainCheck::HistoryUnavailable,
            detail: format!("{} could not be written: {error}", path.display()),
        };
        std::fs::create_dir_all(&self.dir).map_err(|error| unavailable(error, &self.dir))?;
        let target = self.path(&digest);
        let temp = self
            .dir
            .join(format!(".{digest}.{}.new", uuid::Uuid::new_v4().simple()));
        let written = (|| {
            use std::io::Write;
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            std::fs::rename(&temp, &target)
        })();
        if let Err(error) = written {
            let _ = std::fs::remove_file(&temp);
            return Err(unavailable(error, &target));
        }
        Ok((digest, true))
    }

    /// Store `bytes` only when their digest is one of `named`: the digests
    /// the run's launch pin and the current pin's lineage name.
    pub fn import(
        &self,
        bytes: &[u8],
        named: &BTreeSet<String>,
    ) -> Result<(String, bool), ChainRefusal> {
        let digest = content_digest(bytes);
        if !named.contains(&digest) {
            return refuse(
                ChainCheck::UnnamedDigest,
                format!(
                    "blake3 {digest} is not a contract or skeleton digest the launch pin or the pin's lineage names ({})",
                    named.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            );
        }
        self.put(bytes)
    }
}

/// Every contract and skeleton digest the launch pin and `pin`'s lineage
/// name: the only digests an import may file.
pub fn named_digests(
    launch: &PortableAcceptanceIdentityV1,
    pin: &AcceptancePin,
) -> BTreeSet<String> {
    let identities =
        std::iter::once(launch).chain(pin.lineage.iter().flat_map(|link| [&link.from, &link.to]));
    identities
        .flat_map(|identity| {
            std::iter::once(identity.acceptance_digest.clone())
                .chain(identity.skeleton_digest.clone())
        })
        .collect()
}
