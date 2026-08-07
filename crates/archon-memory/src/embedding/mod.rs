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
#[derive(Default)]
pub enum EmbeddingProviderKind {
    /// Automatically select: OpenAI if API key is present, else local.
    #[default]
    Auto,
    /// fastembed BGE-base-en-v1.5 quantized (768-dim, CPU-only).
    Local,
    /// OpenAI-compatible endpoint (default: OpenAI text-embedding-3-small,
    /// 1536-dim; requires API key). ARCHON_MEMORY_EMBEDDING_BASE_URL /
    /// OPENAI_BASE_URL and ARCHON_MEMORY_EMBEDDING_MODEL redirect it to any
    /// OpenAI-compatible proxy.
    #[serde(rename = "openai")]
    OpenAI,
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
    /// API root for the openai provider (e.g. "http://127.0.0.1:1234/v1").
    /// `None` means the real OpenAI API. The ARCHON_MEMORY_EMBEDDING_BASE_URL /
    /// OPENAI_BASE_URL environment variables take precedence over this value.
    pub base_url: Option<String>,
    /// Model name for the openai provider. `None` means text-embedding-3-small.
    /// The ARCHON_MEMORY_EMBEDDING_MODEL environment variable takes precedence.
    pub model: Option<String>,
    /// Intra-op threads for the local provider's ONNX session. `None` takes the
    /// default cap; the value is process-wide and fixed by the first provider
    /// built, since memory and the LEANN index share one session.
    pub intra_threads: Option<usize>,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: EmbeddingProviderKind::Auto,
            hybrid_alpha: 0.3,
            base_url: None,
            model: None,
            intra_threads: None,
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
///
/// The openai provider resolves its endpoint and model env-first with a
/// config fallback: ARCHON_MEMORY_EMBEDDING_BASE_URL (else OPENAI_BASE_URL,
/// else `config.base_url`) and ARCHON_MEMORY_EMBEDDING_MODEL (else
/// `config.model`), so it can target any OpenAI-compatible endpoint (e.g. a
/// local LiteLLM proxy) instead of api.openai.com. Switching endpoint/model
/// changes the embedding space — reindex stored memories afterwards
/// (`archon memory reindex --all`).
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
            let (base_url, model) = resolve_openai_options(config);
            let provider = openai::OpenAIEmbedding::with_options(&key, base_url, model)?;
            Ok(Arc::new(provider))
        }
        EmbeddingProviderKind::Auto => {
            if let Some(key) = openai_api_key() {
                let (base_url, model) = resolve_openai_options(config);
                match openai::OpenAIEmbedding::with_options(&key, base_url, model) {
                    Ok(provider) => return Ok(Arc::new(provider)),
                    Err(e) => {
                        tracing::warn!("OpenAI provider init failed, falling back to local: {e}");
                    }
                }
            }
            local_provider(config)
        }
        EmbeddingProviderKind::Local => local_provider(config),
    }
}

/// The process-wide local embedder, applying `config`'s thread cap if this is
/// the first caller to express one.
///
/// Every local consumer routes through here -- memory and the LEANN code index
/// alike -- so they share one ONNX session rather than loading BGE-base twice.
fn local_provider(config: &EmbeddingConfig) -> Result<Arc<dyn EmbeddingProvider>, MemoryError> {
    let intra_threads = local::configure_intra_threads(config.intra_threads);
    let provider = local::shared()?;
    tracing::debug!(intra_threads, "using the shared local embedding session");
    Ok(provider)
}

/// Endpoint/model for the openai provider: environment wins, config falls back.
fn resolve_openai_options(config: &EmbeddingConfig) -> (Option<String>, Option<String>) {
    (
        openai::env_base_url().or_else(|| config.base_url.clone()),
        openai::env_model().or_else(|| config.model.clone()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_base_url_and_model_used_when_env_absent() {
        // SAFETY: no other test in this binary reads these variables mid-test.
        unsafe {
            std::env::remove_var("ARCHON_MEMORY_EMBEDDING_BASE_URL");
            std::env::remove_var("OPENAI_BASE_URL");
            std::env::remove_var("ARCHON_MEMORY_EMBEDDING_MODEL");
        }
        let config = EmbeddingConfig {
            base_url: Some("http://127.0.0.1:1234/v1".into()),
            model: Some("embed-v4".into()),
            ..Default::default()
        };
        let (base_url, model) = resolve_openai_options(&config);
        assert_eq!(base_url.as_deref(), Some("http://127.0.0.1:1234/v1"));
        assert_eq!(model.as_deref(), Some("embed-v4"));

        let (no_base, no_model) = resolve_openai_options(&EmbeddingConfig::default());
        assert_eq!(no_base, None);
        assert_eq!(no_model, None);
    }

    #[test]
    fn config_without_new_keys_still_parses() {
        let config: EmbeddingConfig =
            serde_json::from_str(r#"{"provider":"local","hybrid_alpha":0.5}"#).expect("parse");
        assert_eq!(config.provider, EmbeddingProviderKind::Local);
        assert_eq!(config.base_url, None);
        assert_eq!(config.model, None);
    }
}
