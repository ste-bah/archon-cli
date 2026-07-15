use std::collections::HashMap;

use super::RawVectorRecord;

#[cfg(test)]
thread_local! {
    static HIT_RESOLUTION_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
pub(super) fn reset_hit_resolution_probes() {
    HIT_RESOLUTION_PROBES.with(|probes| probes.set(0));
}

#[cfg(test)]
pub(super) fn hit_resolution_probes() -> usize {
    HIT_RESOLUTION_PROBES.with(|probes| probes.get())
}
