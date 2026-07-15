use std::collections::HashMap;

use anyhow::Result;

use super::{
    DocVectorStore, HnswSearchHit, build_hnsw_index, generation, persisted_hnsw, validate_provider,
};

impl DocVectorStore {
    pub fn search_persisted_first(
        &self,
        provider: &str,
        query: &[f32],
        top_k: usize,
        ef: usize,
        limit: Option<usize>,
    ) -> Result<Vec<HnswSearchHit>> {
        validate_provider(provider)?;
        let _snapshot_fence = self
            .snapshot_fence
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        #[cfg(test)]
        super::test_hooks::wait_at_snapshot_fence();

        let raw_count = self.count_vectors(Some(provider))?;
        let provider_generation = generation::current(&self.db, provider)?;
        let manifest = self.latest_hnsw_manifest(provider)?;
        if let Some(manifest) = manifest.as_ref()
            && manifest.dimension == query.len()
            && manifest.vector_count == raw_count
            && manifest.provider_generation == Some(provider_generation)
        {
            return persisted_hnsw::search(
                self.hnsw_dir(provider),
                manifest.clone(),
                self.chunk_ids_by_hnsw_id(provider)?,
                query.to_vec(),
                top_k,
                ef,
            );
        }
        persisted_hnsw::clear();
        self.search_in_memory_locked(provider, query, top_k, ef, limit)
    }

    pub fn search_in_memory(
        &self,
        provider: &str,
        query: &[f32],
        top_k: usize,
        ef: usize,
        limit: Option<usize>,
    ) -> Result<Vec<HnswSearchHit>> {
        validate_provider(provider)?;
        let _snapshot_fence = self
            .snapshot_fence
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.search_in_memory_locked(provider, query, top_k, ef, limit)
    }

    fn search_in_memory_locked(
        &self,
        provider: &str,
        query: &[f32],
        top_k: usize,
        ef: usize,
        limit: Option<usize>,
    ) -> Result<Vec<HnswSearchHit>> {
        let records = self.iter_records(Some(provider), limit)?;
        if records.is_empty() || top_k == 0 {
            return Ok(Vec::new());
        }
        let chunk_ids: HashMap<_, _> = super::hnsw_ids::chunk_ids_by_hnsw_id(&records);
        let mut hnsw = build_hnsw_index(&records, query.len())?;
        hnsw.set_searching_mode(true);
        let hits = hnsw.search(query, top_k, ef.max(top_k));
        Ok(hits
            .into_iter()
            .filter_map(|hit| {
                super::chunk_id_for_hnsw_id(&chunk_ids, hit.get_origin_id()).map(|chunk_id| {
                    HnswSearchHit {
                        chunk_id: chunk_id.clone(),
                        distance: hit.get_distance(),
                    }
                })
            })
            .collect())
    }
}
