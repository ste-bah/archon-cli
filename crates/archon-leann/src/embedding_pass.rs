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

/// A deterministic embedding provider: each text hashes to a point on the unit
/// sphere.
///
/// It used to return zero vectors, which is not a neutral choice (issue #145).
/// Cosine distance between two zero vectors is `0/0` -- NaN -- so every pair of
/// points is mutually equidistant and HNSW construction at `m = 50,
/// ef_construction = 200` has nothing to prune on: each insert degenerates
/// towards scanning the graph it has built so far. The cost is superlinear and
/// was measured as such: 226 chunks took 59s and 454 took 206s, which puts a
/// realistically sized corpus out of reach of any test that uses this provider,
/// and pushes single transactions past the store's 60s budget.
///
/// The requirement is only that distinct chunks are not all equidistant, so a
/// hash-seeded direction is enough and keeps the provider reproducible run to
/// run and machine to machine -- which is what "deterministic" was for. Nothing
/// here models semantics: two similar chunks land no closer than two unrelated
/// ones, so this stays useless for asserting search *relevance*, exactly as the
/// zero-vector version was.
struct MockEmbeddingProvider {
    dim: usize,
}

impl EmbeddingProvider for MockEmbeddingProvider {
    fn embed(
        &self,
        texts: &[String],
    ) -> std::result::Result<Vec<Vec<f32>>, archon_memory::types::MemoryError> {
        Ok(texts
            .iter()
            .map(|text| unit_vector_from_text(text, self.dim))
            .collect())
    }

    fn dimensions(&self) -> usize {
        self.dim
    }
}

/// Hash `text` to a direction and normalise it onto the unit sphere.
///
/// Normalising is not cosmetic: cosine distance is undefined at the origin and
/// numerically poor near it, and a unit vector is also what a real embedder
/// hands back, so the stored rows look like production rows.
///
/// The all-zero draw is astronomically unlikely but not impossible, and it is
/// precisely the degenerate case being fixed, so it falls back to a basis
/// vector rather than dividing by zero.
fn unit_vector_from_text(text: &str, dim: usize) -> Vec<f32> {
    let mut state = fnv1a64(text.as_bytes());
    let mut vector: Vec<f32> = (0..dim)
        .map(|_| {
            state = splitmix64(state);
            // Top 24 bits, mapped to [-1, 1): the low bits of splitmix64 are the
            // weakest, and f32 cannot hold more than 24 bits of mantissa anyway.
            ((state >> 40) as f32 / (1u32 << 23) as f32) - 1.0
        })
        .collect();
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    } else if let Some(first) = vector.first_mut() {
        *first = 1.0;
    }
    vector
}

/// FNV-1a, for seeding only -- it just has to spread distinct texts apart.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

/// SplitMix64: the standard finaliser, used here as the per-component stream.
fn splitmix64(state: u64) -> u64 {
    let mut z = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
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
