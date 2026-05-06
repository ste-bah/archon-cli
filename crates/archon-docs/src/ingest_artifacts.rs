use cozo::DbInstance;

use crate::chunking::{build_chunk_artifacts, chunk_with_page_anchors};
use crate::embed;
use crate::errors::DocsError;
use crate::hash::sha256_str;
use crate::models::{ArtifactRecord, ChunkArtifact, PageOffset, ProvenanceEdgeType};
use crate::provenance::make_edge;
use crate::retrieval;
use crate::store;

pub(crate) fn persist_text_artifact_chunks(
    db: &DbInstance,
    document_id: &str,
    artifact_id: &str,
    artifact_type: &str,
    text: &str,
    page_offsets: &[PageOffset],
    chunk_id_prefix: Option<&str>,
) -> Result<Vec<ChunkArtifact>, DocsError> {
    let artifact = ArtifactRecord {
        artifact_id: artifact_id.to_string(),
        document_id: document_id.to_string(),
        artifact_type: artifact_type.to_string(),
        content_hash: sha256_str(text),
        created_at: chrono::Utc::now().to_rfc3339(),
        provenance_record_id: String::new(),
    };
    store::insert_artifact(db, &artifact).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    let page_chunks = chunk_with_page_anchors(text, page_offsets);
    let chunks = match chunk_id_prefix {
        None => build_chunk_artifacts(document_id, artifact_id, &page_chunks),
        Some(prefix) => page_chunks
            .iter()
            .enumerate()
            .map(|(i, page_chunk)| ChunkArtifact {
                chunk_id: format!("chunk-{}-{}", prefix, i),
                document_id: document_id.to_string(),
                artifact_id: artifact_id.to_string(),
                chunk_index: i as u32,
                page_start: page_chunk.page_start,
                page_end: page_chunk.page_end,
                content: page_chunk.content.clone(),
                content_hash: sha256_str(&page_chunk.content),
                embedding_status: "pending".to_string(),
            })
            .collect(),
    };

    for chunk in &chunks {
        store::insert_chunk(db, chunk).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
    }

    store::insert_provenance_edge(
        db,
        &make_edge(artifact_id, document_id, ProvenanceEdgeType::DerivedFrom),
    )
    .map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    Ok(chunks)
}

pub(crate) fn index_chunks_if_provider_available(db: &DbInstance, chunks: &[ChunkArtifact]) {
    if embed::get_provider().is_some() {
        for chunk in chunks {
            if let Err(e) = retrieval::index_chunk(db, chunk) {
                tracing::warn!(
                    chunk_id = %chunk.chunk_id,
                    error = %e,
                    "failed to index PDF-derived chunk during ingest"
                );
            }
        }
    }
}
