//! Local embedding provider abstraction per TSPEC-ARCHON-EVIDENCE-ENGINE-001 §6.2.
//!
//! Supports `search_document:` prefix for stored chunks and `search_query:`
//! prefix for queries. All vectors are L2-normalised for cosine retrieval.
//! The default implementation uses fastembed (ONNX-backed).

pub use crate::embed_config::{EmbeddingProviderConfig, EmbeddingProviderSelection};
use crate::embed_fastembed::{FastembedProvider, MultiFastembedProvider};
use crate::errors::DocsError;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Local embedding provider. All implementations must be `Send + Sync`.
pub trait LocalEmbeddingProvider: Send + Sync {
    /// Embed a batch of document chunks with `search_document:` prefix.
    /// Returns L2-normalised vectors, one per chunk.
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError>;

    /// Embed a single query with `search_query:` prefix.
    /// Returns a single L2-normalised vector.
    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError>;

    /// Embed raw image bytes if this provider is multimodal.
    /// Text-only providers return `Ok(None)` so ingest can skip image
    /// vectors with an explicit warning instead of pretending they exist.
    fn embed_image(&self, _image_bytes: &[u8]) -> Result<Option<Vec<f32>>, DocsError> {
        Ok(None)
    }

    /// Vector dimension produced by this provider.
    fn dimension(&self) -> usize;

    /// Human-readable backend name for status reporting.
    fn backend_name(&self) -> &'static str;

    /// Maximum useful parallel embedding batches for this provider instance.
    ///
    /// Local fastembed is one worker unless explicitly initialised with
    /// multiple model instances. HTTP-compatible providers can run multiple
    /// requests concurrently, so they override this with a higher ceiling.
    fn max_embedding_workers(&self) -> usize {
        1
    }
}

// ---------------------------------------------------------------------------
// Model status
// ---------------------------------------------------------------------------

/// Snapshot of the current embedding backend state.
#[derive(Clone, Debug)]
pub struct ModelStatus {
    pub backend: String,
    pub dimension: usize,
    pub model_name: String,
    pub configured: bool,
}

// ---------------------------------------------------------------------------
// L2 normalisation
// ---------------------------------------------------------------------------

fn l2_norm(v: &[f32]) -> f32 {
    let sum_sq: f32 = v.iter().map(|x| x * x).sum();
    sum_sq.sqrt().max(1e-12)
}

pub(crate) fn normalise(v: &[f32]) -> Vec<f32> {
    let norm = l2_norm(v);
    v.iter().map(|x| x / norm).collect()
}

#[cfg(test)]
fn normalise_batch(vectors: &[Vec<f32>]) -> Vec<Vec<f32>> {
    vectors.iter().map(|v| normalise(v)).collect()
}

// ---------------------------------------------------------------------------
// Provider registry
// ---------------------------------------------------------------------------

use std::sync::{Arc, RwLock};

static PROVIDER: RwLock<Option<Arc<dyn LocalEmbeddingProvider>>> = RwLock::new(None);

/// Get a handle to the global embedding provider.
/// Returns `None` if no provider has been set.
pub fn get_provider() -> Option<Arc<dyn LocalEmbeddingProvider>> {
    PROVIDER.read().ok().and_then(|guard| guard.clone())
}

/// Set the global embedding provider. Replaces any existing provider.
pub fn set_provider(provider: Box<dyn LocalEmbeddingProvider>) {
    if let Ok(mut guard) = PROVIDER.write() {
        if let Some(ref existing) = *guard {
            tracing::warn!(
                new_backend = provider.backend_name(),
                existing_backend = existing.backend_name(),
                "replacing existing embedding provider"
            );
        }
        *guard = Some(Arc::from(provider));
    }
}

/// Try to set the provider, returning an error if one is already set.
/// Use this for production code paths; use `set_provider` for tests.
pub fn try_set_provider(provider: Box<dyn LocalEmbeddingProvider>) -> Result<(), DocsError> {
    if let Ok(mut guard) = PROVIDER.write() {
        if let Some(ref existing) = *guard {
            return Err(DocsError::Validation {
                message: format!(
                    "provider already set: {} (new: {})",
                    existing.backend_name(),
                    provider.backend_name()
                ),
            });
        }
        *guard = Some(Arc::from(provider));
    }
    Ok(())
}

/// Remove the global embedding provider (for testing).
#[cfg(test)]
pub fn clear_provider() {
    if let Ok(mut guard) = PROVIDER.write() {
        *guard = None;
    }
}

static LAST_INIT_ERROR: RwLock<Option<String>> = RwLock::new(None);

/// Return the last init error message, if any.
pub fn last_init_error() -> Option<String> {
    LAST_INIT_ERROR.read().ok().and_then(|g| g.clone())
}

/// Initialise the default fastembed provider.
pub fn init_default_provider() -> Result<(), DocsError> {
    init_provider(EmbeddingProviderConfig::from_env())
}

pub fn init_provider(config: EmbeddingProviderConfig) -> Result<(), DocsError> {
    let result = match config.selection.clone() {
        EmbeddingProviderSelection::Disabled => Err(DocsError::ModelNotConfigured {
            message: "docs embedding provider disabled".into(),
        }),
        EmbeddingProviderSelection::OpenAiCompatible => init_openai_compatible_provider(&config),
        EmbeddingProviderSelection::Local => init_fastembed_provider(&config),
        EmbeddingProviderSelection::Auto => match config.openai_api_key {
            Some(_) => init_openai_compatible_provider(&config),
            None => init_fastembed_provider(&config),
        },
    };
    record_init_result(result)
}

fn init_fastembed_provider(config: &EmbeddingProviderConfig) -> Result<(), DocsError> {
    let instances = config.local_instances.max(1);
    if instances == 1 {
        FastembedProvider::with_load_timeout(config.local_load_timeout).map(|provider| {
            let _ = try_set_provider(Box::new(provider));
        })
    } else {
        MultiFastembedProvider::with_load_timeout(instances, config.local_load_timeout).map(
            |provider| {
                let _ = try_set_provider(Box::new(provider));
            },
        )
    }
}

fn init_openai_compatible_provider(config: &EmbeddingProviderConfig) -> Result<(), DocsError> {
    let key = config
        .openai_api_key
        .clone()
        .ok_or_else(|| DocsError::ModelNotConfigured {
            message: "OpenAI-compatible docs embedding provider needs an API key".into(),
        })?;
    let provider = crate::embed_openai::OpenAiCompatEmbeddingProvider::new(
        key,
        config.openai_base_url.clone(),
        config.openai_model.clone(),
        config.openai_timeout,
    )?;
    let _ = try_set_provider(Box::new(provider));
    Ok(())
}

fn record_init_result(result: Result<(), DocsError>) -> Result<(), DocsError> {
    match result {
        Ok(()) => {
            if let Ok(mut guard) = LAST_INIT_ERROR.write() {
                *guard = None;
            }
            Ok(())
        }
        Err(error) => {
            if let Ok(mut guard) = LAST_INIT_ERROR.write() {
                *guard = Some(error.to_string());
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_l2_normalisation() {
        let v = vec![3.0_f32, 4.0_f32];
        let n = normalise(&v);
        let norm: f32 = n.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "vector must be unit length");
        // Direction preserved
        assert!((n[0] - 0.6).abs() < 1e-6);
        assert!((n[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn test_l2_normalisation_zero_vector() {
        let v = vec![0.0_f32; 10];
        let n = normalise(&v);
        // Zero vector stays zero (division by epsilon)
        assert!(n.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn test_normalise_batch() {
        let batch = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let normed = normalise_batch(&batch);
        assert_eq!(normed.len(), 2);
        assert!((normed[0][0] - 1.0).abs() < 1e-6);
        assert!((normed[1][1] - 1.0).abs() < 1e-6);
    }

    // ── HIGH #2: model-status reports init failure ───────────────────

    struct FailingProvider;

    impl LocalEmbeddingProvider for FailingProvider {
        fn embed_chunks(&self, _chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
            Err(DocsError::Embedding {
                message: "simulated ONNX tensor mismatch".into(),
            })
        }

        fn embed_query(&self, _query: &str) -> Result<Vec<f32>, DocsError> {
            Err(DocsError::Embedding {
                message: "simulated ONNX tensor mismatch".into(),
            })
        }

        fn dimension(&self) -> usize {
            768
        }

        fn backend_name(&self) -> &'static str {
            "failing-mock"
        }
    }

    #[test]
    fn test_model_status_reports_init_failure() {
        let provider = FailingProvider;
        let result = provider.embed_query("hello");
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("simulated ONNX tensor mismatch"),
            "error should contain the real failure message, got: {}",
            err
        );
    }

    #[test]
    #[serial_test::serial(docs_global_state)]
    fn test_last_init_error_captured_on_failure() {
        clear_provider();
        // Simulate a failed init: write an error into LAST_INIT_ERROR
        *LAST_INIT_ERROR.write().unwrap() = Some("simulated init failure".into());
        assert_eq!(last_init_error(), Some("simulated init failure".into()));
        // Clear on success
        *LAST_INIT_ERROR.write().unwrap() = None;
        assert_eq!(last_init_error(), None);
    }

    struct StubProviderA;
    impl LocalEmbeddingProvider for StubProviderA {
        fn embed_chunks(&self, _: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
            Ok(vec![])
        }
        fn embed_query(&self, _: &str) -> Result<Vec<f32>, DocsError> {
            Ok(vec![])
        }
        fn dimension(&self) -> usize {
            1
        }
        fn backend_name(&self) -> &'static str {
            "stub-a"
        }
    }

    struct StubProviderB;
    impl LocalEmbeddingProvider for StubProviderB {
        fn embed_chunks(&self, _: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
            Ok(vec![])
        }
        fn embed_query(&self, _: &str) -> Result<Vec<f32>, DocsError> {
            Ok(vec![])
        }
        fn dimension(&self) -> usize {
            1
        }
        fn backend_name(&self) -> &'static str {
            "stub-b"
        }
    }

    #[test]
    #[serial_test::serial(docs_global_state)]
    fn test_try_set_provider_rejects_double_set() {
        clear_provider();
        assert!(try_set_provider(Box::new(StubProviderA)).is_ok());
        let err = try_set_provider(Box::new(StubProviderB)).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("provider already set"),
            "expected 'provider already set' in: {msg}"
        );
        assert!(
            msg.contains("stub-a"),
            "expected existing backend name in: {msg}"
        );
    }
}
