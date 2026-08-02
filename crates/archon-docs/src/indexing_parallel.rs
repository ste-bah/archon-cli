use std::cell::RefCell;
use std::sync::Arc;
use std::time::Instant;

use cozo::DbInstance;

use crate::embed::LocalEmbeddingProvider;
use crate::errors::DocsError;
use crate::indexing_options::IndexOptions;
use crate::indexing_progress::{IndexProgress, IndexProgressPhase, emit_progress};
use crate::indexing_result::IndexResult;
use crate::models::ChunkArtifact;

#[cfg(test)]
pub(crate) use crate::indexing_parallel_workers::consume_embedded_batches;

#[derive(Clone, Debug)]
pub(crate) struct EmbeddingBatch {
    pub(crate) index: usize,
    pub(crate) chunks: Vec<ChunkArtifact>,
}

#[derive(Debug)]
pub(crate) struct EmbeddedBatch {
    pub(crate) index: usize,
    pub(crate) chunks: Vec<ChunkArtifact>,
    pub(crate) vectors: Result<Vec<Vec<f32>>, String>,
}

pub(crate) fn index_loaded_chunks_parallel(
    db: &DbInstance,
    chunks: Vec<ChunkArtifact>,
    options: &IndexOptions,
    worker_count: usize,
    mut progress: Option<&mut dyn FnMut(IndexProgress)>,
    persist_snapshot: impl FnOnce(IndexResult) -> Result<IndexResult, DocsError>,
) -> Result<IndexResult, DocsError> {
    let started = Instant::now();
    let provider = crate::embed::get_provider().ok_or_else(|| DocsError::ModelNotConfigured {
        message: "no embedding provider configured".into(),
    })?;
    let batch_size = options.batch_size.max(1);
    let batch_total = chunks.len().div_ceil(batch_size);
    let mut result = IndexResult::default();
    emit_candidates_loaded(&mut progress, batch_total, chunks.len(), started);
    process_parallel_batches(
        db,
        chunks,
        provider.clone(),
        &mut result,
        &mut progress,
        ParallelWork {
            batch_size,
            batch_total,
            workers: worker_count,
            max_in_flight: options.effective_max_in_flight_batches(worker_count),
            writer_batch_size: options.effective_writer_batch_size(),
            started,
        },
    )?;
    let result = persist_snapshot(result)?;
    emit_complete(&mut progress, batch_total, &result, started);
    Ok(result)
}

struct ParallelWork {
    batch_size: usize,
    batch_total: usize,
    workers: usize,
    max_in_flight: usize,
    writer_batch_size: usize,
    started: Instant,
}

fn process_parallel_batches(
    db: &DbInstance,
    chunks: Vec<ChunkArtifact>,
    provider: Arc<dyn LocalEmbeddingProvider>,
    result: &mut IndexResult,
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    work: ParallelWork,
) -> Result<(), DocsError> {
    let result_state = RefCell::new(result);
    let progress_state = RefCell::new(progress);
    let batches = build_pending_batches(
        db,
        chunks,
        provider.as_ref(),
        &result_state,
        &progress_state,
        &work,
    );
    crate::indexing_parallel_workers::consume_embedded_batches(
        provider,
        batches,
        work.workers,
        work.max_in_flight,
        |embedded, provider| {
            write_embedded_batch(
                db,
                embedded,
                provider,
                &result_state,
                &progress_state,
                &work,
            )
        },
    )
}

fn build_pending_batches(
    db: &DbInstance,
    chunks: Vec<ChunkArtifact>,
    provider: &dyn LocalEmbeddingProvider,
    result: &RefCell<&mut IndexResult>,
    progress: &RefCell<&mut Option<&mut dyn FnMut(IndexProgress)>>,
    work: &ParallelWork,
) -> Vec<EmbeddingBatch> {
    chunks
        .chunks(work.batch_size)
        .enumerate()
        .filter_map(|(offset, batch)| {
            let mut result = result.borrow_mut();
            let mut progress = progress.borrow_mut();
            let mut context = crate::indexing_parallel_batches::BatchContext {
                db,
                provider,
                result: &mut result,
                progress: &mut progress,
                writer_batch_size: work.writer_batch_size,
                batch_total: work.batch_total,
                started: work.started,
            };
            crate::indexing_parallel_batches::build_embedding_batch(&mut context, batch, offset + 1)
        })
        .collect()
}

fn write_embedded_batch(
    db: &DbInstance,
    embedded: EmbeddedBatch,
    provider: &dyn LocalEmbeddingProvider,
    result: &RefCell<&mut IndexResult>,
    progress: &RefCell<&mut Option<&mut dyn FnMut(IndexProgress)>>,
    work: &ParallelWork,
) {
    let mut result = result.borrow_mut();
    let mut progress = progress.borrow_mut();
    let mut context = crate::indexing_parallel_batches::BatchContext {
        db,
        provider,
        result: &mut result,
        progress: &mut progress,
        writer_batch_size: work.writer_batch_size,
        batch_total: work.batch_total,
        started: work.started,
    };
    crate::indexing_parallel_batches::write_embedded_batch(&mut context, embedded);
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

#[cfg(test)]
#[path = "indexing_parallel_tests.rs"]
mod tests;
