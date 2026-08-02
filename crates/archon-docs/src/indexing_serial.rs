use std::time::Instant;

use cozo::DbInstance;

use crate::embed::LocalEmbeddingProvider;
use crate::errors::DocsError;
use crate::indexing_adaptive::AdaptiveBatchController;
use crate::indexing_progress::{BatchProgress, IndexProgress, IndexProgressPhase, emit_progress};
use crate::indexing_result::IndexResult;
use crate::indexing_store::{mark_batch_failed, retry_batch_individually, store_batch};
use crate::models::ChunkArtifact;

struct SerialBatchContext<'a, 'b, 'c> {
    db: &'a DbInstance,
    provider: &'a dyn LocalEmbeddingProvider,
    result: &'b mut IndexResult,
    progress: &'b mut Option<&'c mut dyn FnMut(IndexProgress)>,
    writer_batch_size: usize,
    batch_total: usize,
    started: Instant,
}

pub(crate) fn index_serially(
    db: &DbInstance,
    chunks: Vec<ChunkArtifact>,
    options: &crate::indexing::IndexOptions,
    provider: &dyn LocalEmbeddingProvider,
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    started: Instant,
) -> Result<IndexResult, DocsError> {
    let batch_size = options.batch_size.max(1);
    let batch_total = chunks.len().div_ceil(batch_size);
    let mut result = IndexResult::default();
    emit_candidates_loaded(progress, batch_total, chunks.len(), started);
    let context = SerialBatchContext {
        db,
        provider,
        result: &mut result,
        progress,
        batch_total,
        started,
        writer_batch_size: options.effective_writer_batch_size(),
    };
    process_serial_batches(context, chunks, batch_size)?;
    let result = crate::indexing::build_persisted_snapshot(provider, result)?;
    emit_complete(progress, batch_total, &result, started);
    Ok(result)
}

fn process_serial_batches(
    mut context: SerialBatchContext<'_, '_, '_>,
    chunks: Vec<ChunkArtifact>,
    batch_size: usize,
) -> Result<(), DocsError> {
    let mut controller = AdaptiveBatchController::from_initial(batch_size);
    let mut offset = 0;
    let mut batch_index = 0;
    while offset < chunks.len() {
        let take = controller.next_size().min(chunks.len() - offset);
        batch_index += 1;
        index_serial_batch(
            &mut context,
            &chunks[offset..offset + take],
            batch_index,
            &mut controller,
        );
        offset += take;
    }
    Ok(())
}

fn emit_candidates_loaded(
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    batch_total: usize,
    total_chunks: usize,
    started: Instant,
) {
    emit_progress(
        progress,
        IndexProgress {
            phase: IndexProgressPhase::CandidatesLoaded,
            batch_index: 0,
            batch_total,
            batch_size: total_chunks,
            batch_position: 0,
            chunk_id: None,
            indexed: 0,
            failed: 0,
            skipped: 0,
            elapsed: started.elapsed(),
        },
    );
}

fn index_serial_batch(
    context: &mut SerialBatchContext<'_, '_, '_>,
    batch: &[ChunkArtifact],
    index: usize,
    controller: &mut AdaptiveBatchController,
) {
    let batch_started = Instant::now();
    emit_batch_started(context, batch, index);
    embed_serial_batch(context, batch, index, controller, batch_started);
    emit_batch_finished(context, batch, index);
}

fn emit_batch_started(
    context: &mut SerialBatchContext<'_, '_, '_>,
    batch: &[ChunkArtifact],
    index: usize,
) {
    emit_batch_event(context, IndexProgressPhase::BatchStarted, batch, index);
}

fn emit_batch_finished(
    context: &mut SerialBatchContext<'_, '_, '_>,
    batch: &[ChunkArtifact],
    index: usize,
) {
    emit_batch_event(context, IndexProgressPhase::BatchFinished, batch, index);
}

fn embed_serial_batch(
    context: &mut SerialBatchContext<'_, '_, '_>,
    batch: &[ChunkArtifact],
    index: usize,
    controller: &mut AdaptiveBatchController,
    batch_started: Instant,
) {
    let uncached = uncached_batch(
        context.db,
        batch,
        context.provider.backend_name(),
        context.result,
    );
    if uncached.is_empty() {
        controller.observe_success(batch_started.elapsed(), batch.len());
        return;
    }
    match context.provider.embed_chunks(&chunk_texts(&uncached)) {
        Ok(vectors) => store_serial_vectors(
            context,
            &uncached,
            vectors,
            index,
            controller,
            batch_started,
        ),
        Err(error) => retry_serial_batch(context, &uncached, index, controller, error),
    }
}

fn store_serial_vectors(
    context: &mut SerialBatchContext<'_, '_, '_>,
    chunks: &[ChunkArtifact],
    vectors: Vec<Vec<f32>>,
    index: usize,
    controller: &mut AdaptiveBatchController,
    started: Instant,
) {
    if vectors.len() != chunks.len() {
        mark_batch_failed(context.db, chunks, context.result);
        controller.observe_failure();
        tracing::warn!(
            expected = chunks.len(),
            actual = vectors.len(),
            "embedding provider returned wrong vector count"
        );
        return;
    }
    store_batch(
        context.db,
        chunks,
        vectors,
        context.provider.backend_name(),
        context.result,
        context.progress,
        context.writer_batch_size,
        batch_progress(index, context),
    );
    controller.observe_success(started.elapsed(), chunks.len());
}

fn retry_serial_batch(
    context: &mut SerialBatchContext<'_, '_, '_>,
    chunks: &[ChunkArtifact],
    index: usize,
    controller: &mut AdaptiveBatchController,
    error: DocsError,
) {
    controller.observe_failure();
    tracing::warn!(%error, "embedding batch failed");
    emit_batch_event(context, IndexProgressPhase::BatchRetrying, chunks, index);
    retry_batch_individually(
        context.db,
        chunks,
        context.provider,
        context.result,
        context.progress,
        context.writer_batch_size,
        batch_progress(index, context),
    );
}

fn emit_batch_event(
    context: &mut SerialBatchContext<'_, '_, '_>,
    phase: IndexProgressPhase,
    batch: &[ChunkArtifact],
    index: usize,
) {
    emit_progress(
        context.progress,
        IndexProgress {
            phase,
            batch_index: index,
            batch_total: context.batch_total,
            batch_size: batch.len(),
            batch_position: 0,
            chunk_id: None,
            indexed: context.result.indexed,
            failed: context.result.failed,
            skipped: context.result.skipped,
            elapsed: context.started.elapsed(),
        },
    );
}

fn chunk_texts(chunks: &[ChunkArtifact]) -> Vec<String> {
    chunks.iter().map(|chunk| chunk.content.clone()).collect()
}

fn batch_progress(index: usize, context: &SerialBatchContext<'_, '_, '_>) -> BatchProgress {
    BatchProgress {
        index,
        total: context.batch_total,
        started: context.started,
    }
}

fn emit_complete(
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    batch_total: usize,
    result: &IndexResult,
    started: Instant,
) {
    emit_progress(
        progress,
        IndexProgress {
            phase: IndexProgressPhase::Complete,
            batch_index: batch_total,
            batch_total,
            batch_size: 0,
            batch_position: 0,
            chunk_id: None,
            indexed: result.indexed,
            failed: result.failed,
            skipped: result.skipped,
            elapsed: started.elapsed(),
        },
    );
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
            tracing::warn!(%error, "embedding cache batch lookup failed; embedding normally");
            batch.to_vec()
        }
    }
}
