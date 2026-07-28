use std::collections::HashMap;

use anyhow::{Context, Result};
use rust_rocksdb::WriteBatch;

use super::{
    DocVectorStore, ID_PREFIX, REVERSE_ID_PREFIX, RawVectorRecord, reverse_id_key,
    reverse_id_marker_key,
};

#[cfg(test)]
thread_local! {
    static HIT_RESOLUTION_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static REVERSE_SCAN_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl DocVectorStore {
    pub(crate) fn chunk_ids_by_hnsw_id(&self, provider: &str) -> Result<HashMap<usize, String>> {
        if self
            .db
            .get_pinned(reverse_id_marker_key(provider))?
            .is_none()
        {
            self.migrate_reverse_hnsw_ids(provider)?;
        }
        let prefix = format!("{REVERSE_ID_PREFIX}/{provider}/");
        #[cfg(test)]
        REVERSE_SCAN_PROBES.with(|probes| probes.set(probes.get() + 1));
        let mut chunk_ids = HashMap::new();
        for item in self.db.prefix_iterator(prefix.as_bytes()) {
            let (key, value) = item.context("iterate RocksDB reverse HNSW identifiers")?;
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            let Some(hnsw_id_text) = std::str::from_utf8(&key)
                .ok()
                .and_then(|key| key.strip_prefix(&prefix))
            else {
                continue;
            };
            if hnsw_id_text == "ready" {
                continue;
            }
            let hnsw_id = hnsw_id_text
                .parse::<usize>()
                .context("parse RocksDB reverse HNSW identifier")?;
            let chunk_id =
                std::str::from_utf8(&value).context("parse RocksDB reverse HNSW chunk ID")?;
            chunk_ids.entry(hnsw_id).or_insert_with(|| chunk_id.into());
        }
        Ok(chunk_ids)
    }

    pub(super) fn migrate_reverse_hnsw_ids(&self, provider: &str) -> Result<()> {
        let prefix = format!("{ID_PREFIX}/{provider}/");
        let mut batch = WriteBatch::default();
        for item in self.db.prefix_iterator(prefix.as_bytes()) {
            let (key, value) = item.context("iterate legacy RocksDB HNSW identifiers")?;
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            let chunk_id = std::str::from_utf8(&key)
                .ok()
                .and_then(|key| key.strip_prefix(&prefix))
                .context("parse legacy RocksDB HNSW identifier key")?;
            anyhow::ensure!(
                value.len() == std::mem::size_of::<usize>(),
                "HNSW identifier has invalid length for {chunk_id}"
            );
            let hnsw_id = usize::from_be_bytes(value.as_ref().try_into()?);
            batch.put(reverse_id_key(provider, hnsw_id), chunk_id.as_bytes());
        }
        batch.put(reverse_id_marker_key(provider), []);
        self.db
            .write(batch)
            .context("migrate reverse HNSW identifiers")
    }
}

pub(super) fn chunk_ids_by_hnsw_id(records: &[RawVectorRecord]) -> HashMap<usize, String> {
    records
        .iter()
        .fold(HashMap::new(), |mut chunk_ids, record| {
            chunk_ids
                .entry(record.hnsw_id)
                .or_insert_with(|| record.chunk_id.clone());
            chunk_ids
        })
}

pub(super) fn chunk_id_for_hnsw_id(
    chunk_ids: &HashMap<usize, String>,
    hnsw_id: usize,
) -> Option<&String> {
    #[cfg(test)]
    HIT_RESOLUTION_PROBES.with(|probes| probes.set(probes.get() + 1));
    chunk_ids.get(&hnsw_id)
}

#[cfg(test)]
pub(super) fn reverse_scan_probes() -> usize {
    REVERSE_SCAN_PROBES.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(super) fn reset_reverse_scan_probes() {
    REVERSE_SCAN_PROBES.with(|probes| probes.set(0));
}

#[cfg(test)]
pub(super) fn reset_hit_resolution_probes() {
    HIT_RESOLUTION_PROBES.with(|probes| probes.set(0));
}

#[cfg(test)]
pub(super) fn hit_resolution_probes() -> usize {
    HIT_RESOLUTION_PROBES.with(|probes| probes.get())
}
