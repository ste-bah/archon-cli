use cozo::DbInstance;

use crate::embed::LocalEmbeddingProvider;
use crate::indexing_progress::{
    BatchProgress, IndexProgress, IndexProgressPhase, emit_chunk_progress, emit_progress,
};
use crate::indexing_result::IndexResult;
use crate::models::ChunkArtifact;
use crate::store;

pub(crate) fn store_batch(
    db: &DbInstance,
    chunks: &[ChunkArtifact],
    vectors: Vec<Vec<f32>>,
    backend: &str,
    result: &mut IndexResult,
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
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
    let rows = chunks
        .iter()
        .zip(vectors.iter())
        .map(|(chunk, embedding)| store::ChunkEmbeddingInput {
            chunk_id: chunk.chunk_id.as_str(),
            embedding,
        })
        .collect::<Vec<_>>();
    let chunk_refs = chunks.iter().collect::<Vec<_>>();
    let bulk_result = store::insert_chunk_embeddings(db, &rows, backend)
        .and_then(|_| store::update_chunk_embedding_statuses(db, &chunk_refs, "indexed"));
    match bulk_result {
        Ok(()) => result.indexed += chunks.len(),
        Err(error) => {
            tracing::warn!(%error, "bulk embedding store failed; retrying chunks individually");
            store_batch_individually(db, chunks, vectors, backend, result, progress, batch);
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
                    BatchProgress {
                        index: batch.index,
                        total: batch.total,
                        started: batch.started,
                    },
                );
            }
            Ok(vectors) => {
                let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
                result.failed += 1;
                tracing::warn!(
                    chunk_id = %chunk.chunk_id,
                    actual = vectors.len(),
                    "single-chunk embedding returned wrong vector count"
                );
            }
            Err(error) => {
                let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
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
        result.failed += chunks.len();
        return;
    }
    for chunk in chunks {
        let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
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
) {
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
        match store::insert_chunk_embedding(db, &chunk.chunk_id, &embedding, backend) {
            Ok(()) => {
                let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "indexed");
                result.indexed += 1;
            }
            Err(error) => {
                let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
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
