use std::time::Instant;

use cozo::DbInstance;

use crate::embed::LocalEmbeddingProvider;
use crate::indexing_progress::{BatchProgress, IndexProgress, IndexProgressPhase, emit_progress};
use crate::indexing_result::IndexResult;
use crate::indexing_store::{mark_batch_failed, retry_batch_individually, store_batch};
use crate::models::ChunkArtifact;

pub(super) struct BatchContext<'a, 'b, 'c> {
    pub(super) db: &'a DbInstance,
    pub(super) provider: &'a dyn LocalEmbeddingProvider,
    pub(super) result: &'b mut IndexResult,
    pub(super) progress: &'b mut Option<&'c mut dyn FnMut(IndexProgress)>,
    pub(super) writer_batch_size: usize,
    pub(super) batch_total: usize,
    pub(super) started: Instant,
}

pub(super) fn build_embedding_batch(
    context: &mut BatchContext<'_, '_, '_>,
    batch: &[ChunkArtifact],
    batch_index: usize,
) -> Option<crate::indexing_parallel::EmbeddingBatch> {
    emit_batch_started(context, batch_index, batch.len());
    let uncached = uncached_batch(
        context.db,
        batch,
        context.provider.backend_name(),
        context.result,
    );
    if uncached.is_empty() {
        emit_batch_finished(context, batch_index, batch.len());
        return None;
    }
    Some(crate::indexing_parallel::EmbeddingBatch {
        index: batch_index,
        chunks: uncached,
    })
}

pub(super) fn write_embedded_batch(
    context: &mut BatchContext<'_, '_, '_>,
    embedded: crate::indexing_parallel::EmbeddedBatch,
) {
    let batch = batch_progress(embedded.index, context);
    match embedded.vectors {
        Ok(vectors) if vectors.len() == embedded.chunks.len() => store_batch(
            context.db,
            &embedded.chunks,
            vectors,
            context.provider.backend_name(),
            context.result,
            context.progress,
            context.writer_batch_size,
            batch,
        ),
        Ok(vectors) => {
            mark_batch_failed(context.db, &embedded.chunks, context.result);
            tracing::warn!(
                expected = embedded.chunks.len(),
                actual = vectors.len(),
                "embedding provider returned wrong vector count"
            );
        }
        Err(ref error) => retry_embedded_batch(context, &embedded, batch, error),
    }
    emit_batch_finished(context, embedded.index, embedded.chunks.len());
}

fn retry_embedded_batch(
    context: &mut BatchContext<'_, '_, '_>,
    embedded: &crate::indexing_parallel::EmbeddedBatch,
    batch: BatchProgress,
    error: &str,
) {
    tracing::warn!(%error, "parallel embedding batch failed");
    emit_batch_retrying(context, embedded.index, embedded.chunks.len());
    retry_batch_individually(
        context.db,
        &embedded.chunks,
        context.provider,
        context.result,
        context.progress,
        context.writer_batch_size,
        batch,
    );
}

fn emit_batch_started(context: &mut BatchContext<'_, '_, '_>, index: usize, size: usize) {
    emit_batch_event(context, IndexProgressPhase::BatchStarted, index, size);
}

fn emit_batch_retrying(context: &mut BatchContext<'_, '_, '_>, index: usize, size: usize) {
    emit_batch_event(context, IndexProgressPhase::BatchRetrying, index, size);
}

fn emit_batch_finished(context: &mut BatchContext<'_, '_, '_>, index: usize, size: usize) {
    emit_batch_event(context, IndexProgressPhase::BatchFinished, index, size);
}

fn emit_batch_event(
    context: &mut BatchContext<'_, '_, '_>,
    phase: IndexProgressPhase,
    batch_index: usize,
    batch_size: usize,
) {
    emit_progress(
        context.progress,
        IndexProgress {
            phase,
            batch_index,
            batch_total: context.batch_total,
            batch_size,
            batch_position: 0,
            chunk_id: None,
            indexed: context.result.indexed,
            failed: context.result.failed,
            skipped: context.result.skipped,
            elapsed: context.started.elapsed(),
        },
    );
}

fn batch_progress(index: usize, context: &BatchContext<'_, '_, '_>) -> BatchProgress {
    BatchProgress {
        index,
        total: context.batch_total,
        started: context.started,
    }
}

fn uncached_batch(
    db: &DbInstance,
    batch: &[ChunkArtifact],
    provider: &str,
    result: &mut IndexResult,
) -> Vec<ChunkArtifact> {
    match crate::indexing_cache::reuse_cached_embeddings(db, batch, provider) {
        Ok(reused) => {
            result.indexed += reused.hits;
            result.cache_hits += reused.hits;
            reused.misses
        }
        Err(error) => {
            tracing::warn!(%error, "embedding cache batch lookup failed");
            batch.to_vec()
        }
    }
}
