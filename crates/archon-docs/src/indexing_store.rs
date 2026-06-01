use cozo::DbInstance;

use crate::embed::LocalEmbeddingProvider;
use crate::indexing_progress::{
    BatchProgress, IndexProgress, IndexProgressPhase, emit_chunk_progress, emit_progress,
};
use crate::indexing_result::IndexResult;
use crate::models::ChunkArtifact;
use crate::store;
use crate::vector_store::{DocVectorStore, VectorWrite};

pub(crate) fn store_batch(
    db: &DbInstance,
    chunks: &[ChunkArtifact],
    vectors: Vec<Vec<f32>>,
    backend: &str,
    result: &mut IndexResult,
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    writer_batch_size: usize,
    batch: BatchProgress,
) {
    emit_progress(
        progress,
        IndexProgress {
            phase: IndexProgressPhase::BatchStoreStarted,
            batch_index: batch.index,
            batch_total: batch.total,
            batch_size: chunks.len(),
            batch_position: 0,
            chunk_id: None,
            indexed: result.indexed,
            failed: result.failed,
            skipped: result.skipped,
            elapsed: batch.started.elapsed(),
        },
    );
    let vector_store = match DocVectorStore::open_default() {
        Ok(store) => store,
        Err(error) => {
            tracing::warn!(%error, "RocksDB vector store unavailable; retrying chunks individually");
            store_batch_individually(db, chunks, vectors, backend, result, progress, batch, None);
            return;
        }
    };
    for (chunk_group, vector_group) in chunks
        .chunks(writer_batch_size.max(1))
        .zip(vectors.chunks(writer_batch_size.max(1)))
    {
        let rows = vector_writes(chunk_group, vector_group, backend);
        if let Err(error) = vector_store.put_vectors(&rows) {
            tracing::warn!(%error, "bulk vector write failed; retrying chunks individually");
            store_batch_individually(
                db,
                chunk_group,
                vector_group.to_vec(),
                backend,
                result,
                progress,
                batch,
                Some(&vector_store),
            );
            continue;
        }
        if let Err(error) = maybe_write_legacy_cozo_vectors(db, chunk_group, vector_group, backend)
        {
            tracing::warn!(%error, "legacy Cozo vector mirror failed; retrying chunks individually");
            store_batch_individually(
                db,
                chunk_group,
                vector_group.to_vec(),
                backend,
                result,
                progress,
                batch,
                Some(&vector_store),
            );
            continue;
        }
        match mark_indexed_after_vector_write(db, chunk_group) {
            Ok(indexed) => {
                result.indexed += indexed;
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    "bulk status update failed after vector write; retrying status only"
                );
                store_batch_individually(
                    db,
                    chunk_group,
                    Vec::new(),
                    backend,
                    result,
                    progress,
                    batch,
                    None,
                );
            }
        }
    }
    emit_progress(
        progress,
        IndexProgress {
            phase: IndexProgressPhase::BatchStoreFinished,
            batch_index: batch.index,
            batch_total: batch.total,
            batch_size: chunks.len(),
            batch_position: 0,
            chunk_id: None,
            indexed: result.indexed,
            failed: result.failed,
            skipped: result.skipped,
            elapsed: batch.started.elapsed(),
        },
    );
}

pub(crate) fn retry_batch_individually(
    db: &DbInstance,
    chunks: &[ChunkArtifact],
    provider: &dyn LocalEmbeddingProvider,
    result: &mut IndexResult,
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    writer_batch_size: usize,
    batch: BatchProgress,
) {
    for (offset, chunk) in chunks.iter().enumerate() {
        match provider.embed_chunks(std::slice::from_ref(&chunk.content)) {
            Ok(mut vectors) if vectors.len() == 1 => {
                store_batch(
                    db,
                    std::slice::from_ref(chunk),
                    vec![vectors.remove(0)],
                    provider.backend_name(),
                    result,
                    progress,
                    writer_batch_size,
                    BatchProgress {
                        index: batch.index,
                        total: batch.total,
                        started: batch.started,
                    },
                );
            }
            Ok(vectors) => {
                let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
                let _ = crate::index_queue::mark_chunks_failed(db, &[chunk], "wrong vector count");
                result.failed += 1;
                tracing::warn!(
                    chunk_id = %chunk.chunk_id,
                    actual = vectors.len(),
                    "single-chunk embedding returned wrong vector count"
                );
            }
            Err(error) => {
                let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
                let _ = crate::index_queue::mark_chunks_failed(db, &[chunk], &error.to_string());
                result.failed += 1;
                tracing::warn!(chunk_id = %chunk.chunk_id, %error, "single-chunk embedding failed");
            }
        }
        emit_chunk_progress(
            progress,
            IndexProgressPhase::ChunkStoreFinished,
            chunk,
            chunks.len(),
            offset + 1,
            result,
            batch,
        );
    }
}

pub(crate) fn mark_batch_failed(
    db: &DbInstance,
    chunks: &[ChunkArtifact],
    result: &mut IndexResult,
) {
    let chunk_refs = chunks.iter().collect::<Vec<_>>();
    if store::update_chunk_embedding_statuses(db, &chunk_refs, "failed").is_ok() {
        let _ = crate::index_queue::mark_chunks_failed(db, &chunk_refs, "batch embedding failed");
        result.failed += chunks.len();
        return;
    }
    for chunk in chunks {
        let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
        let _ = crate::index_queue::mark_chunks_failed(db, &[chunk], "batch embedding failed");
        result.failed += 1;
    }
}

fn store_batch_individually(
    db: &DbInstance,
    chunks: &[ChunkArtifact],
    vectors: Vec<Vec<f32>>,
    backend: &str,
    result: &mut IndexResult,
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    batch: BatchProgress,
    existing_store: Option<&DocVectorStore>,
) {
    if vectors.is_empty() {
        mark_chunks_individually(db, chunks, result, progress, batch);
        return;
    }
    for (offset, (chunk, embedding)) in chunks.iter().zip(vectors).enumerate() {
        let batch_position = offset + 1;
        emit_chunk_progress(
            progress,
            IndexProgressPhase::ChunkStoreStarted,
            chunk,
            chunks.len(),
            batch_position,
            result,
            batch,
        );
        let store_result = if embedding.is_empty() {
            mark_single_indexed(db, chunk)
        } else {
            let write = [VectorWrite {
                chunk_id: &chunk.chunk_id,
                content_hash: &chunk.content_hash,
                provider: backend,
                embedding: &embedding,
            }];
            match existing_store {
                Some(vector_store) => {
                    write_single_chunk(db, vector_store, chunk, &write, &embedding, backend)
                }
                None => DocVectorStore::open_default().and_then(|vector_store| {
                    write_single_chunk(db, &vector_store, chunk, &write, &embedding, backend)
                }),
            }
        };
        match store_result {
            Ok(()) => {
                result.indexed += 1;
            }
            Err(error) => {
                let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
                let _ = crate::index_queue::mark_chunks_failed(db, &[chunk], &error.to_string());
                result.failed += 1;
                tracing::warn!(chunk_id = %chunk.chunk_id, %error, "chunk embedding store failed");
            }
        }
        emit_chunk_progress(
            progress,
            IndexProgressPhase::ChunkStoreFinished,
            chunk,
            chunks.len(),
            batch_position,
            result,
            batch,
        );
    }
}

fn mark_chunks_individually(
    db: &DbInstance,
    chunks: &[ChunkArtifact],
    result: &mut IndexResult,
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    batch: BatchProgress,
) {
    for (offset, chunk) in chunks.iter().enumerate() {
        let batch_position = offset + 1;
        emit_chunk_progress(
            progress,
            IndexProgressPhase::ChunkStoreStarted,
            chunk,
            chunks.len(),
            batch_position,
            result,
            batch,
        );
        match mark_single_indexed(db, chunk) {
            Ok(()) => result.indexed += 1,
            Err(error) => {
                let _ = store::update_chunk_embedding_statuses(db, &[chunk], "failed");
                let _ = crate::index_queue::mark_chunks_failed(db, &[chunk], &error.to_string());
                result.failed += 1;
                tracing::warn!(chunk_id = %chunk.chunk_id, %error, "chunk status update failed");
            }
        }
        emit_chunk_progress(
            progress,
            IndexProgressPhase::ChunkStoreFinished,
            chunk,
            chunks.len(),
            batch_position,
            result,
            batch,
        );
    }
}

fn mark_indexed_after_vector_write(
    db: &DbInstance,
    chunks: &[ChunkArtifact],
) -> anyhow::Result<usize> {
    let chunk_refs = chunks.iter().collect::<Vec<_>>();
    store::update_chunk_embedding_statuses(db, &chunk_refs, "indexed")?;
    if let Err(error) = crate::index_queue::mark_chunks_indexed(db, &chunk_refs) {
        tracing::warn!(%error, "index queue update failed after bulk store");
    }
    Ok(chunks.len())
}

fn mark_single_indexed(db: &DbInstance, chunk: &ChunkArtifact) -> anyhow::Result<()> {
    store::update_chunk_embedding_statuses(db, &[chunk], "indexed")?;
    if let Err(error) = crate::index_queue::mark_chunks_indexed(db, &[chunk]) {
        tracing::warn!(%error, "index queue update failed after single store");
    }
    Ok(())
}

fn write_single_chunk(
    db: &DbInstance,
    vector_store: &DocVectorStore,
    chunk: &ChunkArtifact,
    write: &[VectorWrite<'_>],
    embedding: &[f32],
    backend: &str,
) -> anyhow::Result<()> {
    vector_store
        .put_vectors(write)
        .and_then(|_| maybe_write_legacy_cozo_vector(db, chunk, embedding, backend))
}

fn vector_writes<'a>(
    chunks: &'a [ChunkArtifact],
    vectors: &'a [Vec<f32>],
    backend: &'a str,
) -> Vec<VectorWrite<'a>> {
    chunks
        .iter()
        .zip(vectors.iter())
        .map(|(chunk, embedding)| VectorWrite {
            chunk_id: &chunk.chunk_id,
            content_hash: &chunk.content_hash,
            provider: backend,
            embedding,
        })
        .collect()
}

fn maybe_write_legacy_cozo_vectors(
    db: &DbInstance,
    chunks: &[ChunkArtifact],
    vectors: &[Vec<f32>],
    backend: &str,
) -> anyhow::Result<()> {
    if !legacy_cozo_vector_write_enabled() {
        return Ok(());
    }
    let rows = chunks
        .iter()
        .zip(vectors.iter())
        .map(|(chunk, embedding)| store::ChunkEmbeddingInput {
            chunk_id: chunk.chunk_id.as_str(),
            embedding,
        })
        .collect::<Vec<_>>();
    store::insert_chunk_embeddings(db, &rows, backend)
}

fn maybe_write_legacy_cozo_vector(
    db: &DbInstance,
    chunk: &ChunkArtifact,
    embedding: &[f32],
    backend: &str,
) -> anyhow::Result<()> {
    if legacy_cozo_vector_write_enabled() {
        store::insert_chunk_embedding(db, &chunk.chunk_id, embedding, backend)?;
    }
    Ok(())
}

fn legacy_cozo_vector_write_enabled() -> bool {
    std::env::var("ARCHON_DOCS_LEGACY_COZO_VECTOR_WRITE")
        .map(|value| matches!(value.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}
