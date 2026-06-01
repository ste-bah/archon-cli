use anyhow::Result;
use cozo::DbInstance;

use crate::models::ChunkArtifact;
use crate::store;
use crate::vector_store::{DocVectorStore, VectorWrite};

#[derive(Clone, Debug, Default)]
pub(crate) struct CacheReuseResult {
    pub hits: usize,
    pub misses: Vec<ChunkArtifact>,
}

pub(crate) fn reuse_cached_embeddings(
    db: &DbInstance,
    chunks: &[ChunkArtifact],
    provider: &str,
) -> Result<CacheReuseResult> {
    let vector_store = DocVectorStore::open_default()?;
    reuse_cached_embeddings_with_store(db, &vector_store, chunks, provider)
}

pub(crate) fn reuse_cached_embeddings_with_store(
    db: &DbInstance,
    vector_store: &DocVectorStore,
    chunks: &[ChunkArtifact],
    provider: &str,
) -> Result<CacheReuseResult> {
    let mut hits = Vec::new();
    let mut embeddings = Vec::new();
    let mut misses = Vec::new();
    for chunk in chunks {
        if chunk.content_hash.is_empty() {
            misses.push(chunk.clone());
            continue;
        }
        match vector_store.cached_embedding(provider, &chunk.content_hash)? {
            Some(embedding) => {
                hits.push(chunk.clone());
                embeddings.push(embedding);
            }
            None => misses.push(chunk.clone()),
        }
    }
    if hits.is_empty() {
        return Ok(CacheReuseResult { hits: 0, misses });
    }

    let writes = hits
        .iter()
        .zip(embeddings.iter())
        .map(|(chunk, embedding)| VectorWrite {
            chunk_id: &chunk.chunk_id,
            content_hash: &chunk.content_hash,
            provider,
            embedding,
        })
        .collect::<Vec<_>>();
    vector_store.put_vectors(&writes)?;
    let hit_refs = hits.iter().collect::<Vec<_>>();
    store::update_chunk_embedding_statuses(db, &hit_refs, "indexed")?;
    crate::index_queue::mark_chunks_indexed(db, &hit_refs)?;
    Ok(CacheReuseResult {
        hits: hits.len(),
        misses,
    })
}

#[cfg(test)]
fn reuse_cached_embedding_with_store(
    db: &DbInstance,
    vector_store: &DocVectorStore,
    chunk: &ChunkArtifact,
    provider: &str,
) -> Result<bool> {
    let result = reuse_cached_embeddings_with_store(
        db,
        vector_store,
        std::slice::from_ref(chunk),
        provider,
    )?;
    Ok(result.hits == 1)
}

#[cfg(test)]
mod tests {
    use cozo::DbInstance;

    use super::*;
    use crate::schema::{ensure_doc_schema, ensure_vec_schema};

    fn chunk(id: &str, status: &str) -> ChunkArtifact {
        ChunkArtifact {
            chunk_id: id.into(),
            document_id: "doc-a".into(),
            artifact_id: "artifact-a".into(),
            chunk_index: 1,
            page_start: 1,
            page_end: 1,
            content: "same text".into(),
            content_hash: "same-hash".into(),
            embedding_status: status.into(),
        }
    }

    #[test]
    fn cache_reuses_same_hash_embedding() {
        let db = DbInstance::new("mem", "", Default::default()).unwrap();
        ensure_doc_schema(&db).unwrap();
        ensure_vec_schema(&db, 2).unwrap();
        let source = chunk("source", "indexed");
        let target = chunk("target", "pending");
        store::insert_chunk(&db, &source).unwrap();
        store::insert_chunk(&db, &target).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let vector_store = DocVectorStore::open(temp.path()).unwrap();
        vector_store
            .put_vectors(&[VectorWrite {
                chunk_id: "source",
                content_hash: &source.content_hash,
                provider: "test",
                embedding: &[0.25, 0.75],
            }])
            .unwrap();
        assert!(reuse_cached_embedding_with_store(&db, &vector_store, &target, "test").unwrap());
        assert!(vector_store.has_vector("test", "target").unwrap());
        assert_eq!(
            store::get_chunk_by_id(&db, "target")
                .unwrap()
                .unwrap()
                .embedding_status,
            "indexed"
        );
    }

    #[test]
    fn cache_reuses_hits_in_bulk_and_keeps_misses() {
        let db = DbInstance::new("mem", "", Default::default()).unwrap();
        ensure_doc_schema(&db).unwrap();
        ensure_vec_schema(&db, 2).unwrap();
        let source = chunk("source", "indexed");
        let target_a = chunk("target-a", "pending");
        let target_b = chunk("target-b", "pending");
        let mut miss = chunk("miss", "pending");
        miss.content_hash = "different-hash".into();
        for chunk in [&source, &target_a, &target_b, &miss] {
            store::insert_chunk(&db, chunk).unwrap();
        }
        let temp = tempfile::tempdir().unwrap();
        let vector_store = DocVectorStore::open(temp.path()).unwrap();
        vector_store
            .put_vectors(&[VectorWrite {
                chunk_id: "source",
                content_hash: &source.content_hash,
                provider: "test",
                embedding: &[0.25, 0.75],
            }])
            .unwrap();

        let result = reuse_cached_embeddings_with_store(
            &db,
            &vector_store,
            &[target_a.clone(), target_b.clone(), miss.clone()],
            "test",
        )
        .unwrap();

        assert_eq!(result.hits, 2);
        assert_eq!(result.misses.len(), 1);
        assert_eq!(result.misses[0].chunk_id, miss.chunk_id);
        assert!(vector_store.has_vector("test", "target-a").unwrap());
        assert!(vector_store.has_vector("test", "target-b").unwrap());
    }
}
