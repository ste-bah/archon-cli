use std::time::Instant;

use cozo::{DbInstance, ScriptMutability};

use crate::embed::get_provider;
use crate::errors::DocsError;
use crate::indexing_progress::{BatchProgress, emit_progress};
use crate::indexing_store::{mark_batch_failed, retry_batch_individually, store_batch};
use crate::models::ChunkArtifact;
use crate::store;

pub use crate::indexing_progress::{IndexProgress, IndexProgressPhase};
pub use crate::indexing_result::IndexResult;

const DEFAULT_BATCH_SIZE: usize = 64;

#[derive(Clone, Debug)]
pub struct IndexOptions {
    pub all: bool,
    pub document_id: Option<String>,
    pub batch_size: usize,
    pub limit: Option<usize>,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            all: false,
            document_id: None,
            batch_size: DEFAULT_BATCH_SIZE,
            limit: None,
        }
    }
}

pub fn count_candidates(db: &DbInstance, options: &IndexOptions) -> Result<usize, DocsError> {
    if options.all && options.document_id.is_none() {
        return store::count_chunks(db).map_err(|e| DocsError::Retrieval {
            message: e.to_string(),
        });
    }
    let script = candidate_count_script(options);
    let params = document_params(options);
    let result = db
        .run_script(&script, params, ScriptMutability::Immutable)
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

fn index_chunks_inner(
    db: &DbInstance,
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

    let chunks = list_candidates(db, options)?;
    let mut result = IndexResult::default();
    let batch_size = options.batch_size.max(1);
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

    for (batch_index, batch) in chunks.chunks(batch_size).enumerate() {
        let batch_number = batch_index + 1;
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
        let texts = batch
            .iter()
            .map(|chunk| chunk.content.clone())
            .collect::<Vec<_>>();
        match provider.embed_chunks(&texts) {
            Ok(vectors) if vectors.len() == batch.len() => {
                store_batch(
                    db,
                    batch,
                    vectors,
                    provider.backend_name(),
                    &mut result,
                    &mut progress,
                    BatchProgress {
                        index: batch_number,
                        total: batch_total,
                        started,
                    },
                );
            }
            Ok(vectors) => {
                mark_batch_failed(db, batch, &mut result);
                tracing::warn!(
                    expected = batch.len(),
                    actual = vectors.len(),
                    "embedding provider returned wrong vector count"
                );
            }
            Err(error) => {
                tracing::warn!(%error, "embedding batch failed");
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
                    batch,
                    provider.as_ref(),
                    &mut result,
                    &mut progress,
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
            return Err(error);
        }
    };
    let Some(embedding) = vectors.into_iter().next() else {
        let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
        return Err(DocsError::Embedding {
            message: "embed_chunks returned empty".into(),
        });
    };
    if let Err(error) =
        store::insert_chunk_embedding(db, &chunk.chunk_id, &embedding, provider.backend_name())
    {
        let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "failed");
        return Err(DocsError::Retrieval {
            message: error.to_string(),
        });
    }
    let _ = store::update_chunk_embedding_status(db, &chunk.chunk_id, "indexed");
    Ok(())
}

fn list_candidates(
    db: &DbInstance,
    options: &IndexOptions,
) -> Result<Vec<ChunkArtifact>, DocsError> {
    let script = candidate_list_script(options);
    let params = document_params(options);
    let result = db
        .run_script(&script, params, ScriptMutability::Immutable)
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
