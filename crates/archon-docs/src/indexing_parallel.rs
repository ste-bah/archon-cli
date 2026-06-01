use std::sync::Arc;
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
    for embedded in embed_batches(provider.clone(), batches, worker_count, max_in_flight) {
        write_embedded_batch(
            db,
            embedded,
            provider.as_ref(),
            &mut result,
            &mut progress,
            writer_batch_size,
            batch_total,
            started,
        );
    }
    emit_complete(&mut progress, batch_total, &result, started);
    Ok(result)
}

pub(crate) fn embed_batches(
    provider: Arc<dyn LocalEmbeddingProvider>,
    batches: Vec<EmbeddingBatch>,
    workers: usize,
    max_in_flight_batches: usize,
) -> Vec<EmbeddedBatch> {
    let width = workers.max(1).min(max_in_flight_batches.max(1));
    if width == 1 {
        return batches
            .into_iter()
            .map(|batch| embed_one(Arc::clone(&provider), batch))
            .collect();
    }

    let mut results = Vec::with_capacity(batches.len());
    for group in batches.chunks(width) {
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(group.len());
            for batch in group.iter().cloned() {
                let provider = Arc::clone(&provider);
                handles.push(scope.spawn(move || embed_one(provider, batch)));
            }
            for handle in handles {
                if let Ok(result) = handle.join() {
                    results.push(result);
                }
            }
        });
    }
    results.sort_by_key(|result| result.index);
    results
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
mod tests {
    use super::*;
    use crate::errors::DocsError;

    struct MockProvider;

    impl LocalEmbeddingProvider for MockProvider {
        fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
            Ok(chunks
                .iter()
                .map(|chunk| vec![chunk.len() as f32, 1.0])
                .collect())
        }

        fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
            Ok(vec![query.len() as f32, 1.0])
        }

        fn dimension(&self) -> usize {
            2
        }

        fn backend_name(&self) -> &'static str {
            "mock"
        }
    }

    fn chunk(id: &str, content: &str) -> ChunkArtifact {
        ChunkArtifact {
            chunk_id: id.into(),
            document_id: "doc-a".into(),
            artifact_id: "artifact-a".into(),
            chunk_index: 1,
            page_start: 1,
            page_end: 1,
            content: content.into(),
            content_hash: format!("hash-{id}"),
            embedding_status: "pending".into(),
        }
    }

    #[test]
    fn parallel_batches_preserve_batch_indices() {
        let provider: Arc<dyn LocalEmbeddingProvider> = Arc::new(MockProvider);
        let batches = vec![
            EmbeddingBatch {
                index: 2,
                chunks: vec![chunk("b", "second")],
            },
            EmbeddingBatch {
                index: 1,
                chunks: vec![chunk("a", "first")],
            },
        ];

        let results = embed_batches(provider, batches, 2, 2);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].index, 1);
        assert_eq!(results[1].index, 2);
        assert_eq!(results[0].vectors.as_ref().unwrap()[0], vec![5.0, 1.0]);
    }
}
