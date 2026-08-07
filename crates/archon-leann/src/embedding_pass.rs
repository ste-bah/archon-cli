//! Turning chunks into embedded chunks, in batches the provider can digest.
//!
//! Split out of `indexer.rs` so that file stays inside the 500-line guard; the
//! indexer owns *what* gets embedded and when it is persisted, this module owns
//! only the batching and the provider handshake.

use std::sync::Arc;

use anyhow::{Context, Result};

use archon_memory::embedding::{self, EmbeddingProvider};

use crate::index_storage::PreparedChunk;
use crate::indexer::{ChunkedFile, EmbeddingConfig, EmbeddingProviderKind};
use crate::metadata::CodeChunk;

/// Maximum chunks per embedding batch.
pub(crate) const EMBED_BATCH_SIZE: usize = 64;

/// A deterministic embedding provider that returns zero vectors.
struct MockEmbeddingProvider {
    dim: usize,
}

impl EmbeddingProvider for MockEmbeddingProvider {
    fn embed(
        &self,
        texts: &[String],
    ) -> std::result::Result<Vec<Vec<f32>>, archon_memory::types::MemoryError> {
        Ok(texts.iter().map(|_| vec![0.0; self.dim]).collect())
    }

    fn dimensions(&self) -> usize {
        self.dim
    }
}

pub(crate) fn create_embedder(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>> {
    match config.provider {
        EmbeddingProviderKind::Mock => Ok(Arc::new(MockEmbeddingProvider {
            dim: config.dimension,
        })),
        EmbeddingProviderKind::Local => embedding::create_provider(&embedding::EmbeddingConfig {
            provider: embedding::EmbeddingProviderKind::Local,
            ..Default::default()
        })
        .context("failed to create local embedding provider"),
        EmbeddingProviderKind::OpenAI => embedding::create_provider(&embedding::EmbeddingConfig {
            provider: embedding::EmbeddingProviderKind::OpenAI,
            ..Default::default()
        })
        .context("failed to create OpenAI embedding provider"),
    }
}

/// Embed a whole group's chunks, batching across file boundaries.
///
/// Batches span files deliberately: most source files chunk into far fewer than
/// `EMBED_BATCH_SIZE` pieces, so per-file batching would hand the provider a
/// stream of short calls and waste most of its throughput. The returned vector
/// is parallel to `files`, so the caller can still persist file by file.
///
/// `Ok(None)` means cancelled, not failed.
pub(crate) fn prepare_repository_files(
    embedder: &Arc<dyn EmbeddingProvider>,
    files: &[ChunkedFile<'_>],
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Vec<Vec<PreparedChunk>>>> {
    let mut prepared = files
        .iter()
        .map(|file| Vec::with_capacity(file.chunks.len()))
        .collect::<Vec<_>>();
    let mut batch = Vec::with_capacity(EMBED_BATCH_SIZE);

    for (file_index, file) in files.iter().enumerate() {
        for chunk in &file.chunks {
            if cancelled() {
                return Ok(None);
            }
            batch.push((file_index, chunk));
            if batch.len() == EMBED_BATCH_SIZE
                && embed_batch(embedder, &mut prepared, &mut batch, cancelled)?.is_none()
            {
                return Ok(None);
            }
        }
    }
    if !batch.is_empty() && embed_batch(embedder, &mut prepared, &mut batch, cancelled)?.is_none() {
        return Ok(None);
    }
    Ok(Some(prepared))
}

fn embed_batch(
    embedder: &Arc<dyn EmbeddingProvider>,
    prepared: &mut [Vec<PreparedChunk>],
    batch: &mut Vec<(usize, &CodeChunk)>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<()>> {
    if cancelled() {
        return Ok(None);
    }
    let texts = batch
        .iter()
        .map(|(_, chunk)| chunk.metadata.chunk_content.clone())
        .collect::<Vec<_>>();
    let embeddings = embed_texts(embedder, &texts, batch.len())?;
    if cancelled() {
        return Ok(None);
    }
    for ((file_index, chunk), embedding) in batch.drain(..).zip(embeddings) {
        prepared[file_index].push(PreparedChunk {
            chunk: chunk.clone(),
            embedding,
        });
    }
    Ok(Some(()))
}

/// Embed one file's chunks. `Ok(None)` means cancelled, not failed.
pub(crate) fn prepare_chunks(
    embedder: &Arc<dyn EmbeddingProvider>,
    chunks: Vec<CodeChunk>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<Vec<PreparedChunk>>> {
    let mut prepared = Vec::with_capacity(chunks.len());
    for batch in chunks.chunks(EMBED_BATCH_SIZE) {
        if cancelled() {
            return Ok(None);
        }
        let texts = batch
            .iter()
            .map(|chunk| chunk.metadata.chunk_content.clone())
            .collect::<Vec<_>>();
        let embeddings = embed_texts(embedder, &texts, batch.len())?;
        if cancelled() {
            return Ok(None);
        }
        prepared.extend(
            batch
                .iter()
                .cloned()
                .zip(embeddings)
                .map(|(chunk, embedding)| PreparedChunk { chunk, embedding }),
        );
    }
    Ok(Some(prepared))
}

/// Embed `texts` and insist the provider returned one vector per input.
///
/// A short or long result would otherwise be zipped against the chunk list and
/// silently pair embeddings with the wrong code.
fn embed_texts(
    embedder: &Arc<dyn EmbeddingProvider>,
    texts: &[String],
    expected: usize,
) -> Result<Vec<Vec<f32>>> {
    let embeddings = embedder
        .embed(texts)
        .map_err(|error| anyhow::anyhow!("embedding failed: {error}"))?;
    if embeddings.len() != expected {
        anyhow::bail!(
            "embedding count mismatch: got {} for {} chunks",
            embeddings.len(),
            expected
        );
    }
    Ok(embeddings)
}
