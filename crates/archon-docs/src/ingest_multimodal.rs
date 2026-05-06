use cozo::DbInstance;

use crate::chunking::chunk_with_page_anchors;
use crate::embed;
use crate::errors::DocsError;
use crate::hash::sha256_str;
use crate::ingest::PipelineOutcome;
use crate::models::{
    ArtifactRecord, ChunkArtifact, ImageDescription, PageOffset, ProvenanceEdgeType,
};
use crate::provenance::make_edge;
use crate::retrieval;
use crate::store;
use crate::vlm::{self, VlmDescriptionOutcome};

pub(crate) async fn apply_vlm_description(
    db: &DbInstance,
    document_id: &str,
    content_bytes: &[u8],
    policy: &archon_policy::EffectivePolicy,
    page_ids: &[String],
    outcome: &mut PipelineOutcome,
) -> Result<(), DocsError> {
    let policy = policy.clone();
    let image_bytes = content_bytes.to_vec();
    let vlm_result =
        tokio::task::spawn_blocking(move || vlm::describe_registered_image(&policy, &image_bytes))
            .await
            .map_err(|e| DocsError::VlmProvider {
                provider: "runtime".into(),
                message: format!("VLM worker join failed: {e}"),
                status_code: None,
            })?;

    match vlm_result {
        Err(e) => {
            outcome
                .warnings
                .push(format!("image description failed: {e}"));
        }
        Ok(VlmDescriptionOutcome::Disabled(reason)) => {
            outcome
                .warnings
                .push(format!("image description skipped: {reason}"));
        }
        Ok(VlmDescriptionOutcome::NoProvider) => {
            outcome
                .warnings
                .push("image description skipped: VLM provider not configured".into());
        }
        Ok(VlmDescriptionOutcome::Described(description)) if description.text.trim().is_empty() => {
            outcome
                .warnings
                .push("image description skipped: provider returned empty description".into());
        }
        Ok(VlmDescriptionOutcome::Described(description)) => {
            persist_vlm_description(db, document_id, page_ids, &description)?;
            outcome.warnings.push(format!(
                "image description ok via {}/{} ({}ms, ${:.4})",
                description.provider,
                description.model,
                description.duration_ms,
                description.cost_usd
            ));
            outcome.vlm_descriptions += 1;
        }
    }

    Ok(())
}

fn persist_vlm_description(
    db: &DbInstance,
    document_id: &str,
    page_ids: &[String],
    description: &vlm::VlmDescription,
) -> Result<(), DocsError> {
    let description_text = description.text.trim();
    let artifact_id = format!("vlm-description-{}", uuid::Uuid::new_v4());
    let created_at = chrono::Utc::now().to_rfc3339();
    let artifact = ArtifactRecord {
        artifact_id: artifact_id.clone(),
        document_id: document_id.to_string(),
        artifact_type: "image_description".to_string(),
        content_hash: sha256_str(description_text),
        created_at: created_at.clone(),
        provenance_record_id: String::new(),
    };
    store::insert_artifact(db, &artifact).map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    store::insert_image_description(
        db,
        &ImageDescription {
            artifact_id: artifact_id.clone(),
            document_id: document_id.to_string(),
            page_number: page_ids
                .first()
                .and_then(|id| page_number_from_id(id))
                .unwrap_or(1),
            provider: description.provider.clone(),
            model: description.model.clone(),
            description: description_text.to_string(),
            created_at,
            cost_usd: description.cost_usd,
        },
    )
    .map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    let page_offsets = vec![PageOffset {
        page: 1,
        char_start: 0,
        char_end: description_text.len(),
    }];
    let page_chunks = chunk_with_page_anchors(description_text, &page_offsets);
    let chunks: Vec<ChunkArtifact> = page_chunks
        .iter()
        .enumerate()
        .map(|(i, page_chunk)| ChunkArtifact {
            chunk_id: format!("chunk-{}-{}", artifact_id, i),
            document_id: document_id.to_string(),
            artifact_id: artifact_id.clone(),
            chunk_index: i as u32,
            page_start: page_chunk.page_start,
            page_end: page_chunk.page_end,
            content: page_chunk.content.clone(),
            content_hash: sha256_str(&page_chunk.content),
            embedding_status: "pending".to_string(),
        })
        .collect();

    for chunk in &chunks {
        store::insert_chunk(db, chunk).map_err(|e| DocsError::Storage {
            message: e.to_string(),
        })?;
        if embed::get_provider().is_some()
            && let Err(e) = retrieval::index_chunk(db, chunk)
        {
            tracing::warn!(
                chunk_id = %chunk.chunk_id,
                error = %e,
                "failed to index VLM description chunk during ingest"
            );
        }
        for page_id in page_ids {
            store::insert_provenance_edge(
                db,
                &make_edge(&chunk.chunk_id, page_id, ProvenanceEdgeType::Describes),
            )
            .map_err(|e| DocsError::Storage {
                message: e.to_string(),
            })?;
        }
    }

    store::insert_provenance_edge(
        db,
        &make_edge(&artifact_id, document_id, ProvenanceEdgeType::DerivedFrom),
    )
    .map_err(|e| DocsError::Storage {
        message: e.to_string(),
    })?;

    Ok(())
}

fn page_number_from_id(page_id: &str) -> Option<u32> {
    page_id.rsplit('-').next()?.parse().ok()
}

pub(crate) fn store_image_embedding_if_supported(
    db: &DbInstance,
    page_ids: &[String],
    content_bytes: &[u8],
    suppress_unsupported_warning: bool,
    outcome: &mut PipelineOutcome,
) {
    let Some(page_id) = page_ids.first() else {
        outcome
            .warnings
            .push("image embedding skipped: no page artifact was created".into());
        return;
    };
    let Some(provider) = embed::get_provider() else {
        outcome
            .warnings
            .push("image embedding skipped: no embedding provider configured".into());
        return;
    };
    if let Err(e) = crate::schema::ensure_vec_schema(db, provider.dimension()) {
        outcome.warnings.push(format!(
            "image embedding skipped: vector schema unavailable: {e}"
        ));
        return;
    }

    match provider.embed_image(content_bytes) {
        Ok(Some(embedding)) => {
            match store::insert_page_image_embedding(
                db,
                page_id,
                &embedding,
                provider.backend_name(),
            ) {
                Ok(()) => outcome.image_embeddings_stored += 1,
                Err(e) => outcome
                    .warnings
                    .push(format!("image embedding skipped: storage failed: {e}")),
            }
        }
        Ok(None) if suppress_unsupported_warning => {}
        Ok(None) => outcome.warnings.push(format!(
            "image embedding skipped: provider {} does not support image embeddings",
            provider.backend_name()
        )),
        Err(e) => outcome
            .warnings
            .push(format!("image embedding skipped: provider failed: {e}")),
    }
}
