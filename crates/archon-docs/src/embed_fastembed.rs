use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::embed::{LocalEmbeddingProvider, normalise};
use crate::errors::DocsError;

const PREFIX_DOCUMENT: &str = "search_document: ";
const PREFIX_QUERY: &str = "search_query: ";
const BGE_BASE_DIM: usize = 768;

pub struct FastembedProvider {
    model: Mutex<Option<fastembed::TextEmbedding>>,
    cache_dir: PathBuf,
    load_timeout: Duration,
}

impl FastembedProvider {
    pub fn with_load_timeout(load_timeout: Duration) -> Result<Self, DocsError> {
        let cache_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("archon")
            .join("fastembed");
        std::fs::create_dir_all(&cache_dir).map_err(|e| DocsError::Embedding {
            message: format!("failed to create fastembed cache dir: {e}"),
        })?;
        Ok(Self {
            model: Mutex::new(None),
            cache_dir,
            load_timeout,
        })
    }

    fn ensure_model(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, Option<fastembed::TextEmbedding>>, DocsError> {
        let mut guard = self.model.lock().map_err(|e| DocsError::Embedding {
            message: format!("embedding model lock poisoned: {e}"),
        })?;
        if guard.is_none() {
            tracing::info!(
                cache_dir = %self.cache_dir.display(),
                "loading local embedding model BGE-base-en-v1.5"
            );
            *guard = Some(load_fastembed_with_timeout(
                self.cache_dir.clone(),
                self.load_timeout,
            )?);
        }
        Ok(guard)
    }
}

impl LocalEmbeddingProvider for FastembedProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        if chunks.is_empty() {
            return Ok(Vec::new());
        }
        let guard = self.ensure_model()?;
        let model = guard.as_ref().ok_or_else(|| DocsError::Embedding {
            message: "model not loaded".into(),
        })?;
        let prefixed = chunks
            .iter()
            .map(|c| format!("{PREFIX_DOCUMENT}{c}"))
            .collect::<Vec<_>>();
        let raw = model
            .embed(prefixed, None)
            .map_err(|e| DocsError::Embedding {
                message: format!("fastembed embed_chunks failed: {e}"),
            })?;
        Ok(normalise_batch(&raw))
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        let guard = self.ensure_model()?;
        let model = guard.as_ref().ok_or_else(|| DocsError::Embedding {
            message: "model not loaded".into(),
        })?;
        let raw = model
            .embed(vec![format!("{PREFIX_QUERY}{query}")], None)
            .map_err(|e| DocsError::Embedding {
                message: format!("fastembed embed_query failed: {e}"),
            })?;
        Ok(normalise(&raw[0]))
    }

    fn dimension(&self) -> usize {
        BGE_BASE_DIM
    }

    fn backend_name(&self) -> &'static str {
        "fastembed-onnx"
    }
}

pub struct MultiFastembedProvider {
    providers: Vec<FastembedProvider>,
    next: AtomicUsize,
}

impl MultiFastembedProvider {
    pub fn with_load_timeout(instances: usize, load_timeout: Duration) -> Result<Self, DocsError> {
        let instances = instances.max(1);
        let mut providers = Vec::with_capacity(instances);
        for _ in 0..instances {
            providers.push(FastembedProvider::with_load_timeout(load_timeout)?);
        }
        Ok(Self {
            providers,
            next: AtomicUsize::new(0),
        })
    }

    fn provider_for_batch(&self) -> &FastembedProvider {
        let index = self.next.fetch_add(1, Ordering::Relaxed) % self.providers.len();
        &self.providers[index]
    }
}

impl LocalEmbeddingProvider for MultiFastembedProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        self.provider_for_batch().embed_chunks(chunks)
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        self.providers[0].embed_query(query)
    }

    fn dimension(&self) -> usize {
        BGE_BASE_DIM
    }

    fn backend_name(&self) -> &'static str {
        "fastembed-onnx"
    }

    fn max_embedding_workers(&self) -> usize {
        self.providers.len()
    }
}

fn normalise_batch(vectors: &[Vec<f32>]) -> Vec<Vec<f32>> {
    vectors.iter().map(|v| normalise(v)).collect()
}

fn load_fastembed_with_timeout(
    cache_dir: PathBuf,
    timeout: Duration,
) -> Result<fastembed::TextEmbedding, DocsError> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let options = fastembed::InitOptions::new(fastembed::EmbeddingModel::BGEBaseENV15)
            .with_cache_dir(cache_dir)
            .with_show_download_progress(false);
        let _ = tx.send(fastembed::TextEmbedding::try_new(options));
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(model)) => Ok(model),
        Ok(Err(error)) => Err(DocsError::Embedding {
            message: format!("failed to load fastembed model: {error}"),
        }),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(DocsError::Embedding {
            message: format!(
                "timed out loading fastembed model after {}s",
                timeout.as_secs()
            ),
        }),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(DocsError::Embedding {
            message: "fastembed model loader thread exited without a result".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_fastembed_reports_instance_parallelism_without_loading_models() {
        let provider = MultiFastembedProvider::with_load_timeout(3, Duration::from_secs(1))
            .expect("provider should initialise wrappers without loading models");
        assert_eq!(provider.max_embedding_workers(), 3);
        assert_eq!(provider.backend_name(), "fastembed-onnx");
    }
}
