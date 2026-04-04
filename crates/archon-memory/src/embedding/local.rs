//! Local embedding provider using fastembed (BGE-base-en-v1.5 quantized).
//!
//! The model is loaded lazily on the first call to [`LocalEmbedding::embed`].
//! Model files are cached under `~/.local/share/archon/fastembed/`.

use std::path::PathBuf;
use std::sync::Mutex;

use crate::types::MemoryError;

use super::EmbeddingProvider;

/// CPU-only local embedding provider (768-dimension vectors).
pub struct LocalEmbedding {
    model: Mutex<Option<fastembed::TextEmbedding>>,
    cache_dir: PathBuf,
}

impl LocalEmbedding {
    /// Create a new local embedding provider.  The model is NOT loaded yet;
    /// it will be initialised lazily on the first `embed()` call.
    pub fn new() -> Result<Self, MemoryError> {
        let cache_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("archon")
            .join("fastembed");
        std::fs::create_dir_all(&cache_dir).map_err(|e| {
            MemoryError::Database(format!("failed to create fastembed cache dir: {e}"))
        })?;
        Ok(Self {
            model: Mutex::new(None),
            cache_dir,
        })
    }

    /// Ensure the model is loaded, returning a reference guard.
    fn ensure_model(&self) -> Result<std::sync::MutexGuard<'_, Option<fastembed::TextEmbedding>>, MemoryError> {
        let mut guard = self.model.lock().map_err(|e| {
            MemoryError::Database(format!("embedding model lock poisoned: {e}"))
        })?;
        if guard.is_none() {
            tracing::info!(
                cache_dir = %self.cache_dir.display(),
                "loading local embedding model BGE-base-en-v1.5 (quantized)"
            );
            let options = fastembed::InitOptions::new(fastembed::EmbeddingModel::BGEBaseENV15Q)
                .with_cache_dir(self.cache_dir.clone())
                .with_show_download_progress(false);
            let model = fastembed::TextEmbedding::try_new(options).map_err(|e| {
                MemoryError::Database(format!("failed to load fastembed model: {e}"))
            })?;
            *guard = Some(model);
        }
        Ok(guard)
    }
}

impl EmbeddingProvider for LocalEmbedding {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let guard = self.ensure_model()?;
        let model = guard.as_ref().ok_or_else(|| {
            MemoryError::Database("embedding model not loaded after init".into())
        })?;
        let results = model.embed(texts.to_vec(), None).map_err(|e| {
            MemoryError::Database(format!("fastembed embed failed: {e}"))
        })?;
        Ok(results)
    }

    fn dimensions(&self) -> usize {
        768
    }
}

// Safety: fastembed::TextEmbedding is Send but not Sync by default.
// We guard it behind a Mutex, making the outer struct safe for Sync.
// The Mutex ensures only one thread accesses the model at a time.
unsafe impl Sync for LocalEmbedding {}
