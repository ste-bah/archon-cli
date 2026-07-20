use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use cozo::DbInstance;

use crate::embed::LocalEmbeddingProvider;
use crate::errors::DocsError;
use crate::indexing_options::IndexOptions;
use crate::indexing_progress::{BatchProgress, IndexProgress, IndexProgressPhase, emit_progress};
use crate::indexing_result::IndexResult;
use crate::indexing_store::{mark_batch_failed, retry_batch_individually, store_batch};
use crate::models::ChunkArtifact;

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

#[derive(Debug)]
struct PermitPool {
    available: Mutex<usize>,
    signal: Condvar,
}

impl PermitPool {
    fn new(permits: usize) -> Self {
        Self {
            available: Mutex::new(permits.max(1)),
            signal: Condvar::new(),
        }
    }

    fn acquire(self: &Arc<Self>) -> Permit {
        let mut available = self.available.lock().expect("permit pool lock");
        while *available == 0 {
            available = self.signal.wait(available).expect("permit pool wait");
        }
        *available -= 1;
        Permit {
            pool: Arc::clone(self),
        }
    }
}

struct Permit {
    pool: Arc<PermitPool>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        *self.pool.available.lock().expect("permit pool lock") += 1;
        self.pool.signal.notify_one();
    }
}

struct LimitedProvider {
    inner: Arc<dyn LocalEmbeddingProvider>,
    permits: Arc<PermitPool>,
}

impl LocalEmbeddingProvider for LimitedProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        let _permit = self.permits.acquire();
        self.inner.embed_chunks(chunks)
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        let _permit = self.permits.acquire();
        self.inner.embed_query(query)
    }

    fn dimension(&self) -> usize {
        self.inner.dimension()
    }

    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
}

pub(crate) fn index_loaded_chunks_parallel(
    db: &DbInstance,
    chunks: Vec<ChunkArtifact>,
    options: &IndexOptions,
    worker_count: usize,
    mut progress: Option<&mut dyn FnMut(IndexProgress)>,
) -> Result<IndexResult, DocsError> {
    let started = Instant::now();
    let provider = crate::embed::get_provider().ok_or_else(|| DocsError::ModelNotConfigured {
        message: "no embedding provider configured".into(),
    })?;
    let batch_size = options.batch_size.max(1);
    let batch_total = chunks.len().div_ceil(batch_size);
    let max_in_flight = options.effective_max_in_flight_batches(worker_count);
    let writer_batch_size = options.effective_writer_batch_size();
    let mut result = IndexResult::default();
    emit_candidates_loaded(&mut progress, batch_total, chunks.len(), started);

    let batches = build_embedding_batches(
        db,
        &chunks,
        batch_size,
        batch_total,
        provider.backend_name(),
        &mut result,
        &mut progress,
        started,
    );
    consume_embedded_batches(
        provider,
        batches,
        worker_count,
        max_in_flight,
        |embedded, provider| {
            write_embedded_batch(
                db,
                embedded,
                provider,
                &mut result,
                &mut progress,
                writer_batch_size,
                batch_total,
                started,
            );
        },
    )?;
    emit_complete(&mut progress, batch_total, &result, started);
    Ok(result)
}

fn consume_embedded_batches(
    provider: Arc<dyn LocalEmbeddingProvider>,
    batches: Vec<EmbeddingBatch>,
    workers: usize,
    max_in_flight_batches: usize,
    mut consume: impl FnMut(EmbeddedBatch, &dyn LocalEmbeddingProvider),
) -> Result<(), DocsError> {
    let worker_count = workers.max(1);
    let max_in_flight = max_in_flight_batches.max(1);
    let residence_permits = Arc::new(PermitPool::new(max_in_flight));
    let provider: Arc<dyn LocalEmbeddingProvider> = Arc::new(LimitedProvider {
        inner: provider,
        permits: Arc::new(PermitPool::new(worker_count)),
    });
    let queue = Arc::new(Mutex::new(VecDeque::from(batches)));
    let (sender, receiver) = std::sync::mpsc::sync_channel(max_in_flight);
    let worker_result = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let provider = Arc::clone(&provider);
            let queue = Arc::clone(&queue);
            let sender = sender.clone();
            let residence_permits = Arc::clone(&residence_permits);
            handles.push(scope.spawn(move || {
                loop {
                    let permit = residence_permits.acquire();
                    let batch = queue
                        .lock()
                        .expect("embedding batch queue lock")
                        .pop_front();
                    let Some(batch) = batch else {
                        break;
                    };
                    if sender
                        .send((embed_one(Arc::clone(&provider), batch), permit))
                        .is_err()
                    {
                        break;
                    }
                }
            }));
        }
        drop(sender);
        for (embedded, _permit) in receiver {
            consume(embedded, provider.as_ref());
        }
        for handle in handles {
            handle.join().map_err(|_| ())?;
        }
        Ok(())
    });
    worker_result.map_err(|()| DocsError::Embedding {
        message: "embedding worker panicked".into(),
    })
}

fn embed_one(provider: Arc<dyn LocalEmbeddingProvider>, batch: EmbeddingBatch) -> EmbeddedBatch {
    let texts = batch
        .chunks
        .iter()
        .map(|chunk| chunk.content.clone())
        .collect::<Vec<_>>();
    EmbeddedBatch {
        index: batch.index,
        chunks: batch.chunks,
        vectors: provider
            .embed_chunks(&texts)
            .map_err(|error| error.to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_embedding_batches(
    db: &DbInstance,
    chunks: &[ChunkArtifact],
    batch_size: usize,
    batch_total: usize,
    provider: &str,
    result: &mut IndexResult,
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    started: Instant,
) -> Vec<EmbeddingBatch> {
    let mut batches = Vec::new();
    for (offset, batch) in chunks.chunks(batch_size).enumerate() {
        let batch_index = offset + 1;
        emit_batch_started(
            progress,
            batch_index,
            batch_total,
            batch.len(),
            result,
            started,
        );
        let uncached = uncached_batch(db, batch, provider, result);
        if uncached.is_empty() {
            emit_batch_finished(
                progress,
                batch_index,
                batch_total,
                batch.len(),
                result,
                started,
            );
        } else {
            batches.push(EmbeddingBatch {
                index: batch_index,
                chunks: uncached,
            });
        }
    }
    batches
}

#[allow(clippy::too_many_arguments)]
fn write_embedded_batch(
    db: &DbInstance,
    embedded: EmbeddedBatch,
    provider: &dyn LocalEmbeddingProvider,
    result: &mut IndexResult,
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    writer_batch_size: usize,
    batch_total: usize,
    started: Instant,
) {
    let batch = BatchProgress {
        index: embedded.index,
        total: batch_total,
        started,
    };
    match embedded.vectors {
        Ok(vectors) if vectors.len() == embedded.chunks.len() => {
            store_batch(
                db,
                &embedded.chunks,
                vectors,
                provider.backend_name(),
                result,
                progress,
                writer_batch_size,
                batch,
            );
        }
        Ok(vectors) => {
            mark_batch_failed(db, &embedded.chunks, result);
            tracing::warn!(
                expected = embedded.chunks.len(),
                actual = vectors.len(),
                "embedding provider returned wrong vector count"
            );
        }
        Err(error) => {
            tracing::warn!(%error, "parallel embedding batch failed");
            emit_batch_retrying(
                progress,
                embedded.index,
                batch_total,
                embedded.chunks.len(),
                result,
                started,
            );
            retry_batch_individually(
                db,
                &embedded.chunks,
                provider,
                result,
                progress,
                writer_batch_size,
                batch,
            );
        }
    }
    emit_batch_finished(
        progress,
        embedded.index,
        batch_total,
        embedded.chunks.len(),
        result,
        started,
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
            tracing::warn!(%error, "embedding cache batch lookup failed");
            batch.to_vec()
        }
    }
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

fn emit_batch_started(
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    batch_index: usize,
    batch_total: usize,
    batch_size: usize,
    result: &IndexResult,
    started: Instant,
) {
    emit_batch_event(
        progress,
        IndexProgressPhase::BatchStarted,
        batch_index,
        batch_total,
        batch_size,
        result,
        started,
    );
}

fn emit_batch_retrying(
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    batch_index: usize,
    batch_total: usize,
    batch_size: usize,
    result: &IndexResult,
    started: Instant,
) {
    emit_batch_event(
        progress,
        IndexProgressPhase::BatchRetrying,
        batch_index,
        batch_total,
        batch_size,
        result,
        started,
    );
}

fn emit_batch_finished(
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    batch_index: usize,
    batch_total: usize,
    batch_size: usize,
    result: &IndexResult,
    started: Instant,
) {
    emit_batch_event(
        progress,
        IndexProgressPhase::BatchFinished,
        batch_index,
        batch_total,
        batch_size,
        result,
        started,
    );
}

fn emit_complete(
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    batch_total: usize,
    result: &IndexResult,
    started: Instant,
) {
    emit_batch_event(
        progress,
        IndexProgressPhase::Complete,
        batch_total,
        batch_total,
        0,
        result,
        started,
    );
}

fn emit_batch_event(
    progress: &mut Option<&mut dyn FnMut(IndexProgress)>,
    phase: IndexProgressPhase,
    batch_index: usize,
    batch_total: usize,
    batch_size: usize,
    result: &IndexResult,
    started: Instant,
) {
    emit_progress(
        progress,
        IndexProgress {
            phase,
            batch_index,
            batch_total,
            batch_size,
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
