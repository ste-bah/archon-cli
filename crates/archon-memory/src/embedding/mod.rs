//! Dual embedding provider system: local (fastembed) and OpenAI.
//!
//! The [`create_provider`] factory selects the appropriate backend based on
//! configuration and available environment variables.

pub mod local;
pub mod openai;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::types::MemoryError;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Synchronous embedding provider (both local and OpenAI are blocking).
pub trait EmbeddingProvider: Send + Sync {
    /// Compute embeddings for a batch of texts.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError>;

    /// The dimensionality of vectors produced by this provider.
    fn dimensions(&self) -> usize;
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Which embedding backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingProviderKind {
    /// Automatically select: OpenAI if API key is present, else local.
    Auto,
    /// fastembed BGE-base-en-v1.5 quantized (768-dim, CPU-only).
    Local,
    /// OpenAI text-embedding-3-small (1536-dim, requires API key).
    #[serde(rename = "openai")]
    OpenAI,
}

impl Default for EmbeddingProviderKind {
    fn default() -> Self {
        Self::Auto
    }
}

impl std::fmt::Display for EmbeddingProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => f.write_str("auto"),
            Self::Local => f.write_str("local"),
            Self::OpenAI => f.write_str("openai"),
        }
    }
}

/// Configuration for the embedding subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EmbeddingConfig {
    /// Which provider to use: `auto`, `local`, or `openai`.
    pub provider: EmbeddingProviderKind,
    /// Weight of keyword score in hybrid search (0.0 = pure vector, 1.0 = pure keyword).
    pub hybrid_alpha: f32,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: EmbeddingProviderKind::Auto,
            hybrid_alpha: 0.3,
        }
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Look up the OpenAI API key from the environment.
fn openai_api_key() -> Option<String> {
    std::env::var("ARCHON_MEMORY_OPENAIKEY")
        .ok()
        .or_else(|| std::env::var("OPENAI_API_KEY").ok())
        .filter(|k| !k.is_empty())
}

/// Create an embedding provider based on configuration.
///
/// - `"auto"`: use OpenAI if an API key is found in the environment, else local.
/// - `"local"`: always use local fastembed.
/// - `"openai"`: require an API key or return an error.
pub fn create_provider(
    config: &EmbeddingConfig,
) -> Result<Arc<dyn EmbeddingProvider>, MemoryError> {
    match config.provider {
        EmbeddingProviderKind::OpenAI => {
            let key = openai_api_key().ok_or_else(|| {
                MemoryError::Database(
                    "OpenAI embedding provider requested but no API key found. \
                     Set OPENAI_API_KEY or ARCHON_MEMORY_OPENAIKEY."
                        .into(),
                )
            })?;
            let provider = openai::OpenAIEmbedding::new(&key)?;
            Ok(Arc::new(provider))
        }
        EmbeddingProviderKind::Auto => {
            if let Some(key) = openai_api_key() {
                match openai::OpenAIEmbedding::new(&key) {
                    Ok(provider) => return Ok(Arc::new(provider)),
                    Err(e) => {
                        tracing::warn!("OpenAI provider init failed, falling back to local: {e}");
                    }
                }
            }
            let provider = local::LocalEmbedding::new()?;
            Ok(Arc::new(provider))
        }
        EmbeddingProviderKind::Local => {
            let provider = local::LocalEmbedding::new()?;
            Ok(Arc::new(provider))
        }
    }
}
