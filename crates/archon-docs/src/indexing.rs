use std::time::Instant;

use cozo::{DbInstance, ScriptMutability};

use crate::embed::get_provider;
use crate::errors::DocsError;
use crate::indexing_adaptive::AdaptiveBatchController;
use crate::indexing_progress::{BatchProgress, emit_progress};
use crate::indexing_store::{mark_batch_failed, retry_batch_individually, store_batch};
use crate::models::ChunkArtifact;
use crate::store;

pub use crate::indexing_options::IndexOptions;
pub use crate::indexing_progress::{IndexProgress, IndexProgressPhase};
pub use crate::indexing_result::IndexResult;

pub fn count_candidates(db: &DbInstance, options: &IndexOptions) -> Result<usize, DocsError> {
    if options.all && options.document_id.is_none() {
        return store::count_chunks(db).map_err(|e| DocsError::Retrieval {
            message: e.to_string(),
        });
    }
    let script = candidate_count_script(options);
    let params = document_params(options);
    let result = crate::cozo_retry::run_script_guarded(
        db,
        &script,
        params,
        ScriptMutability::Immutable,
        "count index candidates",
    )
    .map_err(|e| DocsError::Retrieval {
        message: format!("count index candidates failed: {e}"),
    })?;
    Ok(result
        .rows
        .first()
        .and_then(|row| row.first())
        .and_then(|value| value.get_int())
        .unwrap_or(0) as usize)
}

pub fn index_chunks(db: &DbInstance, options: &IndexOptions) -> Result<IndexResult, DocsError> {
    index_chunks_inner(db, options, None)
}

pub fn index_chunks_with_progress<F>(
    db: &DbInstance,
    options: &IndexOptions,
    mut progress: F,
) -> Result<IndexResult, DocsError>
where
    F: FnMut(IndexProgress),
{
    index_chunks_inner(db, options, Some(&mut progress))
}

pub fn index_loaded_chunks_with_progress<F>(
    db: &DbInstance,
    chunks: Vec<ChunkArtifact>,
    batch_size: usize,
    mut progress: F,
) -> Result<IndexResult, DocsError>
where
    F: FnMut(IndexProgress),
{
    let options = IndexOptions {
        batch_size,
        ..Default::default()
    };
    index_loaded_chunks_inner(db, chunks, &options, Some(&mut progress))
}

pub fn index_loaded_chunks_with_options_progress<F>(
    db: &DbInstance,
    chunks: Vec<ChunkArtifact>,
    options: &IndexOptions,
    mut progress: F,
) -> Result<IndexResult, DocsError>
where
    F: FnMut(IndexProgress),
{
    index_loaded_chunks_inner(db, chunks, options, Some(&mut progress))
}

fn index_chunks_inner(
    db: &DbInstance,
    options: &IndexOptions,
    progress: Option<&mut dyn FnMut(IndexProgress)>,
) -> Result<IndexResult, DocsError> {
    let chunks = list_candidates(db, options)?;
    index_loaded_chunks_inner(db, chunks, options, progress)
}

fn index_loaded_chunks_inner(
    db: &DbInstance,
    chunks: Vec<ChunkArtifact>,
    options: &IndexOptions,
    progress: Option<&mut dyn FnMut(IndexProgress)>,
) -> Result<IndexResult, DocsError> {
    let mut progress = progress;
    let started = Instant::now();
    let provider = get_provider().ok_or_else(|| DocsError::ModelNotConfigured {
        message: "no embedding provider configured".into(),
    })?;
    crate::schema::ensure_vec_schema(db, provider.dimension()).map_err(|e| {
        DocsError::Retrieval {
            message: format!("failed to ensure vec schema: {e}"),
        }
    })?;

    let mut result = IndexResult::default();
    let provider_name = provider.backend_name();
    let requested_workers = options.effective_embedding_workers(provider_name);
    let worker_count = requested_workers
        .min(provider.max_embedding_workers().max(1))
        .max(1);
    if requested_workers > worker_count {
        tracing::warn!(
            requested_workers,
            worker_count,
            provider = provider_name,
            "embedding worker count capped by provider parallelism"
        );
    }
    if worker_count > 1 {
        return crate::indexing_parallel::index_loaded_chunks_parallel(
            db,
            chunks,
            options,
            worker_count,
            progress,
        );
    }

    let batch_size = options.batch_size.max(1);
    let writer_batch_size = options.effective_writer_batch_size();
    let mut controller = AdaptiveBatchController::from_initial(batch_size);
    let batch_total = chunks.len().div_ceil(batch_size);
    emit_progress(
        &mut progress,
        IndexProgress {
            phase: IndexProgressPhase::CandidatesLoaded,
            batch_index: 0,
            batch_total,
            batch_size: chunks.len(),
            batch_position: 0,
            chunk_id: None,
            indexed: 0,
            failed: 0,
            skipped: 0,
            elapsed: started.elapsed(),
        },
    );

    let mut offset = 0;
    let mut batch_number = 0;
    while offset < chunks.len() {
        let take = controller.next_size().min(chunks.len() - offset);
        let batch = &chunks[offset..offset + take];
        let batch_started = Instant::now();
        batch_number += 1;
        emit_progress(
            &mut progress,
            IndexProgress {
                phase: IndexProgressPhase::BatchStarted,
                batch_index: batch_number,
                batch_total,
                batch_size: batch.len(),
                batch_position: 0,
                chunk_id: None,
                indexed: result.indexed,
                failed: result.failed,
                skipped: result.skipped,
                elapsed: started.elapsed(),
            },
        );
        let batch = uncached_batch(db, batch, provider.backend_name(), &mut result);
        if batch.is_empty() {
            controller.observe_success(batch_started.elapsed(), take);
            emit_progress(
                &mut progress,
                IndexProgress {
                    phase: IndexProgressPhase::BatchFinished,
                    batch_index: batch_number,
                    batch_total,
                    batch_size: take,
                    batch_position: 0,
                    chunk_id: None,
                    indexed: result.indexed,
                    failed: result.failed,
                    skipped: result.skipped,
                    elapsed: started.elapsed(),
                },
            );
            offset += take;
            continue;
        }
        let texts = batch
            .iter()
            .map(|chunk| chunk.content.clone())
            .collect::<Vec<_>>();
        match provider.embed_chunks(&texts) {
            Ok(vectors) if vectors.len() == batch.len() => {
                store_batch(
                    db,
                    &batch,
                    vectors,
                    provider.backend_name(),
                    &mut result,
                    &mut progress,
                    writer_batch_size,
                    BatchProgress {
                        index: batch_number,
                        total: batch_total,
                        started,
                    },
                );
                controller.observe_success(batch_started.elapsed(), batch.len());
            }
            Ok(vectors) => {
                mark_batch_failed(db, &batch, &mut result);
                controller.observe_failure();
                tracing::warn!(
                    expected = batch.len(),
                    actual = vectors.len(),
                    "embedding provider returned wrong vector count"
                );
            }
            Err(error) => {
                tracing::warn!(%error, "embedding batch failed");
                controller.observe_failure();
                emit_progress(
                    &mut progress,
                    IndexProgress {
                        phase: IndexProgressPhase::BatchRetrying,
                        batch_index: batch_number,
                        batch_total,
                        batch_size: batch.len(),
                        batch_position: 0,
                        chunk_id: None,
                        indexed: result.indexed,
                        failed: result.failed,
                        skipped: result.skipped,
                        elapsed: started.elapsed(),
                    },
                );
                retry_batch_individually(
                    db,
                    &batch,
                    provider.as_ref(),
                    &mut result,
                    &mut progress,
                    writer_batch_size,
                    BatchProgress {
                        index: batch_number,
                        total: batch_total,
                        started,
                    },
                );
            }
        }
        emit_progress(
            &mut progress,
            IndexProgress {
                phase: IndexProgressPhase::BatchFinished,
                batch_index: batch_number,
                batch_total,
                batch_size: batch.len(),
                batch_position: 0,
                chunk_id: None,
                indexed: result.indexed,
                failed: result.failed,
                skipped: result.skipped,
                elapsed: started.elapsed(),
            },
        );
        offset += take;
    }
    emit_progress(
        &mut progress,
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
    Ok(result)
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

pub fn index_chunk(db: &DbInstance, chunk: &ChunkArtifact) -> Result<(), DocsError> {
    let provider = get_provider().ok_or_else(|| DocsError::ModelNotConfigured {
        message: "no embedding provider configured".into(),
    })?;
    crate::schema::ensure_vec_schema(db, provider.dimension()).map_err(|e| {
        DocsError::Retrieval {
            message: format!("failed to ensure vec schema: {e}"),
        }
    })?;
    let vectors = match provider.embed_chunks(&[chunk.content.clone()]) {
        Ok(vectors) => vectors,
        Err(error) => {
            let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
            let _ = crate::index_queue::mark_chunks_failed(db, &[chunk], &error.to_string());
            return Err(error);
        }
    };
    let Some(embedding) = vectors.into_iter().next() else {
        let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
        let _ = crate::index_queue::mark_chunks_failed(db, &[chunk], "embed_chunks returned empty");
        return Err(DocsError::Embedding {
            message: "embed_chunks returned empty".into(),
        });
    };
    let vector_store = match crate::vector_store::DocVectorStore::open_default() {
        Ok(store) => store,
        Err(error) => {
            let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
            let _ = crate::index_queue::mark_chunks_failed(db, &[chunk], &error.to_string());
            return Err(DocsError::Retrieval {
                message: error.to_string(),
            });
        }
    };
    let write = [crate::vector_store::VectorWrite {
        chunk_id: &chunk.chunk_id,
        content_hash: &chunk.content_hash,
        provider: provider.backend_name(),
        embedding: &embedding,
    }];
    if let Err(error) = vector_store.put_vectors(&write) {
        let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
        let _ = crate::index_queue::mark_chunks_failed(db, &[chunk], &error.to_string());
        return Err(DocsError::Retrieval {
            message: error.to_string(),
        });
    }
    let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "indexed");
    let _ = crate::index_queue::mark_chunks_indexed(db, &[chunk]);
    Ok(())
}

fn list_candidates(
    db: &DbInstance,
    options: &IndexOptions,
) -> Result<Vec<ChunkArtifact>, DocsError> {
    let script = candidate_list_script(options);
    let params = document_params(options);
    let result = crate::cozo_retry::run_script_guarded(
        db,
        &script,
        params,
        ScriptMutability::Immutable,
        "list index candidates",
    )
    .map_err(|e| DocsError::Retrieval {
        message: format!("list index candidates failed: {e}"),
    })?;
    Ok(result.rows.iter().map(|row| chunk_from_row(row)).collect())
}

fn candidate_count_script(options: &IndexOptions) -> String {
    match (options.all, options.document_id.is_some()) {
        (true, true) => {
            "?[count(chunk_id)] := *doc_chunks{chunk_id, document_id}, document_id = $document_id"
                .into()
        }
        (true, false) => "?[count(chunk_id)] := *doc_chunks{chunk_id}".into(),
        (false, true) => {
            "?[count(chunk_id)] := *doc_chunks{chunk_id, document_id, embedding_status}, \
             document_id = $document_id, embedding_status = \"pending\""
                .into()
        }
        (false, false) => "?[count(chunk_id)] := *doc_chunks{chunk_id, embedding_status}, \
             embedding_status = \"pending\""
            .into(),
    }
}

fn candidate_list_script(options: &IndexOptions) -> String {
    let mut script = match (options.all, options.document_id.is_some()) {
        (true, true) => {
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}, \
             document_id = $document_id"
                .to_string()
        }
        (true, false) => {
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}"
                .to_string()
        }
        (false, true) => {
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}, \
             document_id = $document_id, embedding_status = \"pending\""
                .to_string()
        }
        (false, false) => {
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             := *doc_chunks{chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status}, \
             embedding_status = \"pending\""
                .to_string()
        }
    };
    script.push_str(" :order document_id, chunk_index");
    if let Some(limit) = options.limit {
        script.push_str(&format!(" :limit {limit}"));
    }
    script
}

fn document_params(options: &IndexOptions) -> std::collections::BTreeMap<String, cozo::DataValue> {
    let mut params = std::collections::BTreeMap::new();
    if let Some(document_id) = &options.document_id {
        params.insert(
            "document_id".into(),
            cozo::DataValue::from(document_id.as_str()),
        );
    }
    params
}

fn chunk_from_row(row: &[cozo::DataValue]) -> ChunkArtifact {
    ChunkArtifact {
        chunk_id: row[0].get_str().unwrap_or("").to_string(),
        document_id: row[1].get_str().unwrap_or("").to_string(),
        artifact_id: row[2].get_str().unwrap_or("").to_string(),
        chunk_index: row[3].get_int().unwrap_or(0) as u32,
        page_start: row[4].get_int().unwrap_or(0) as u32,
        page_end: row[5].get_int().unwrap_or(0) as u32,
        content: row[6].get_str().unwrap_or("").to_string(),
        content_hash: row[7].get_str().unwrap_or("").to_string(),
        embedding_status: row[8].get_str().unwrap_or("pending").to_string(),
    }
}
