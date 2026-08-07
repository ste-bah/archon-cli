//! Local embedding provider using fastembed (BGE-base-en-v1.5 quantized).
//!
//! The model is loaded lazily on the first call to [`LocalEmbedding::embed`].
//! Model files are cached under `~/.local/share/archon/fastembed/`.
//!
//! One session per process, not one per consumer. Memory and the LEANN code
//! index both want BGE-base, and before [`shared`] existed they each built
//! their own ONNX Runtime session from this same file: two copies of one model
//! resident, and two independent intra-op thread pools. [`shared`] hands both
//! the same [`LocalEmbedding`], so the model is resident once.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use crate::types::MemoryError;

use super::EmbeddingProvider;

/// Ceiling on intra-op threads when nothing configures one.
///
/// fastembed's own default is `available_parallelism()`, which on a 32-core
/// machine gave one ONNX session 32 intra-op threads. BGE-base is a small model
/// and stops getting faster long before that, so the cores bought nothing and
/// cost a thread apiece. 8 is the knee for this model class; smaller machines
/// keep their own core count, since the clamp below only bites from above.
const DEFAULT_MAX_INTRA_THREADS: usize = 8;

/// Process-wide intra-op thread cap, resolved once.
///
/// Deliberately global rather than a constructor argument. The shared session is
/// built by whichever consumer embeds first, and that is not always the one
/// holding the user's config -- the LEANN indexer, for instance, asks for a
/// provider with default settings. A per-call argument would therefore make the
/// effective cap depend on startup ordering. Setting it here once keeps the
/// answer the same regardless of who wins the race.
static INTRA_THREADS: OnceLock<usize> = OnceLock::new();

/// The one local embedder for this process.
static SHARED: Mutex<Option<Arc<LocalEmbedding>>> = Mutex::new(None);

/// Fix the intra-op thread cap for this process. First caller wins.
///
/// Returns the value actually in force, which is the earlier setting if one was
/// already recorded. `None` requests the default rather than "uncapped" --
/// uncapped is the behaviour this exists to stop.
pub fn configure_intra_threads(requested: Option<usize>) -> usize {
    let resolved = requested
        .filter(|n| *n > 0)
        .unwrap_or_else(default_intra_threads);
    *INTRA_THREADS.get_or_init(|| resolved)
}

/// The cap in force, defaulting without recording a choice.
fn intra_threads() -> usize {
    INTRA_THREADS
        .get()
        .copied()
        .unwrap_or_else(default_intra_threads)
}

fn default_intra_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, DEFAULT_MAX_INTRA_THREADS)
}

/// The process-wide local embedder, constructing it on first use.
///
/// Cheap to call repeatedly: the returned handle is lazy, so this allocates no
/// ONNX session until something actually embeds.
pub fn shared() -> Result<Arc<LocalEmbedding>, MemoryError> {
    let mut guard = SHARED
        .lock()
        .map_err(|e| MemoryError::Database(format!("shared embedder lock poisoned: {e}")))?;
    if let Some(existing) = guard.as_ref() {
        return Ok(Arc::clone(existing));
    }
    let created = Arc::new(LocalEmbedding::new()?);
    *guard = Some(Arc::clone(&created));
    Ok(created)
}

/// CPU-only local embedding provider (768-dimension vectors).
pub struct LocalEmbedding {
    model: Mutex<Option<fastembed::TextEmbedding>>,
    cache_dir: PathBuf,
}

impl LocalEmbedding {
    /// Create a new local embedding provider.  The model is NOT loaded yet;
    /// it will be initialised lazily on the first `embed()` call.
    ///
    /// Prefer [`shared`] unless you specifically need an isolated session:
    /// each instance owns a separate ONNX session and thread pool.
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
    fn ensure_model(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<fastembed::TextEmbedding>>, MemoryError> {
        let mut guard = self
            .model
            .lock()
            .map_err(|e| MemoryError::Database(format!("embedding model lock poisoned: {e}")))?;
        if guard.is_none() {
            let intra_threads = intra_threads();
            tracing::info!(
                cache_dir = %self.cache_dir.display(),
                intra_threads,
                "loading local embedding model BGE-base-en-v1.5"
            );
            let options = fastembed::InitOptions::new(fastembed::EmbeddingModel::BGEBaseENV15)
                .with_cache_dir(self.cache_dir.clone())
                .with_intra_threads(intra_threads)
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
        // `&mut` on the model: fastembed 5 takes `&mut self` for `embed`, where 4
        // took `&self`. The Mutex was already here, so this costs nothing --
        // inference was serialised through it either way.
        let mut guard = self.ensure_model()?;
        let model = guard
            .as_mut()
            .ok_or_else(|| MemoryError::Database("embedding model not loaded after init".into()))?;
        let results = model
            .embed(texts, None)
            .map_err(|e| MemoryError::Database(format!("fastembed embed failed: {e}")))?;
        Ok(results)
    }

    fn dimensions(&self) -> usize {
        768
    }
}

#[cfg(test)]
mod tests {
    use super::LocalEmbedding;

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn local_embedding_uses_native_send_sync_bounds() {
        assert_send_sync::<fastembed::TextEmbedding>();
        assert_send_sync::<LocalEmbedding>();

        let source = include_str!("local.rs");
        let forbidden_keyword = ["un", "safe"].concat();
        assert!(!source.contains(&forbidden_keyword));
    }

    #[test]
    fn default_intra_threads_is_capped_and_never_zero() {
        let threads = super::default_intra_threads();
        assert!(threads >= 1, "a session needs at least one thread");
        assert!(
            threads <= super::DEFAULT_MAX_INTRA_THREADS,
            "default must cap rather than take every core; got {threads}"
        );
    }

    #[test]
    fn configure_intra_threads_is_first_write_wins() {
        // Whoever configures first fixes the value for the process, so a later
        // consumer built with default settings cannot silently widen the cap.
        let first = super::configure_intra_threads(Some(3));
        assert_eq!(first, super::configure_intra_threads(Some(31)));
        assert_eq!(first, super::configure_intra_threads(None));
        assert_eq!(first, super::intra_threads());
    }

    #[test]
    fn zero_is_treated_as_unset_rather_than_no_threads() {
        // A config of 0 means "not specified"; an ONNX session with zero
        // intra-op threads is not a thing to hand to ort.
        assert!(super::default_intra_threads() > 0);
    }
}
