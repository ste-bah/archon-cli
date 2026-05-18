//! Embedding adapter interface for world-model state/action text.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddingBackendKind {
    Local,
    External,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbeddingRequest {
    pub text: String,
    pub source_hash: String,
    pub redaction_policy: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingVector {
    pub values: Vec<f32>,
    pub provider: String,
    pub model: String,
    pub source_hash: String,
    pub redaction_policy: String,
}

pub trait WorldEmbeddingAdapter: Send + Sync {
    fn backend_kind(&self) -> EmbeddingBackendKind;
    fn dimensions(&self) -> usize;
    fn provider_name(&self) -> &str;
    fn model_name(&self) -> &str;
    fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingVector>;
}

pub struct MemoryEmbeddingAdapter {
    provider: Arc<dyn archon_memory::embedding::EmbeddingProvider>,
    backend_kind: EmbeddingBackendKind,
    provider_name: String,
    model_name: String,
    projection_dimensions: usize,
}

impl MemoryEmbeddingAdapter {
    pub fn local_fastembed(projection_dimensions: usize) -> Result<Self> {
        let provider = archon_memory::embedding::local::LocalEmbedding::new()
            .map_err(|error| anyhow::anyhow!("local fastembed init failed: {error}"))?;
        Ok(Self {
            provider: Arc::new(provider),
            backend_kind: EmbeddingBackendKind::Local,
            provider_name: "fastembed".into(),
            model_name: "bge-base-en-v1.5".into(),
            projection_dimensions,
        })
    }

    pub fn openai(
        projection_dimensions: usize,
        allow_third_party: bool,
        api_key_env: Option<&str>,
    ) -> Result<Self> {
        if !allow_third_party {
            bail!("third-party embeddings are disabled by config/policy");
        }
        let api_key = api_key_env
            .and_then(|name| std::env::var(name).ok())
            .or_else(|| std::env::var("ARCHON_MEMORY_OPENAIKEY").ok())
            .or_else(|| std::env::var("OPENAI_API_KEY").ok())
            .ok_or_else(|| anyhow::anyhow!("OpenAI embedding API key is unavailable"))?;
        let provider = archon_memory::embedding::openai::OpenAIEmbedding::new(&api_key)
            .map_err(|error| anyhow::anyhow!("OpenAI embedding init failed: {error}"))?;
        Ok(Self {
            provider: Arc::new(provider),
            backend_kind: EmbeddingBackendKind::External,
            provider_name: "openai".into(),
            model_name: "text-embedding-3-small".into(),
            projection_dimensions,
        })
    }
}

impl WorldEmbeddingAdapter for MemoryEmbeddingAdapter {
    fn backend_kind(&self) -> EmbeddingBackendKind {
        self.backend_kind.clone()
    }

    fn dimensions(&self) -> usize {
        self.projection_dimensions
    }

    fn provider_name(&self) -> &str {
        &self.provider_name
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingVector> {
        let vectors = self
            .provider
            .embed(std::slice::from_ref(&request.text))
            .map_err(|error| anyhow::anyhow!("world-model embedding failed: {error}"))?;
        let source = vectors
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("embedding provider returned no vectors"))?;
        Ok(EmbeddingVector {
            values: project_vector(&source, self.projection_dimensions),
            provider: self.provider_name.clone(),
            model: self.model_name.clone(),
            source_hash: request.source_hash.clone(),
            redaction_policy: request.redaction_policy.clone(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicHashEmbeddingAdapter {
    dimensions: usize,
}

impl DeterministicHashEmbeddingAdapter {
    pub fn new(dimensions: usize) -> Result<Self> {
        if dimensions == 0 {
            bail!("embedding dimensions must be greater than zero");
        }
        Ok(Self { dimensions })
    }
}

impl WorldEmbeddingAdapter for DeterministicHashEmbeddingAdapter {
    fn backend_kind(&self) -> EmbeddingBackendKind {
        EmbeddingBackendKind::Local
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn provider_name(&self) -> &str {
        "local"
    }

    fn model_name(&self) -> &str {
        "deterministic-hash-v1"
    }

    fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingVector> {
        let mut values = vec![0.0; self.dimensions];
        for token in request.text.split_whitespace() {
            let mut hasher = DefaultHasher::new();
            token.hash(&mut hasher);
            let hash = hasher.finish();
            let bucket = (hash as usize) % self.dimensions;
            let sign = if hash & 1 == 0 { 1.0 } else { -1.0 };
            values[bucket] += sign;
        }
        normalize(&mut values);

        Ok(EmbeddingVector {
            values,
            provider: "local".into(),
            model: "deterministic-hash-v1".into(),
            source_hash: request.source_hash.clone(),
            redaction_policy: request.redaction_policy.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingCacheConfig {
    pub cache_dir: PathBuf,
    pub cache_enabled: bool,
    pub cache_max_bytes: u64,
    pub redact_before_embedding: bool,
    /// Schema version used to invalidate all cached keys when the eval schema changes.
    /// Bump this to force a cold cache on all keys.
    /// TODO(T025): read from config.learning.world_model.jepa.eval.eval_schema_version
    pub eval_schema_version: u32,
}

pub struct CachedEmbeddingAdapter {
    inner: Box<dyn WorldEmbeddingAdapter>,
    config: EmbeddingCacheConfig,
    /// Resolved from WorldModelPolicy.allow_embedding_cache at construction time.
    /// When false, write_cached() is skipped (read-through only — policy denies persistence).
    allow_cache: bool,
}

/// On-disk record for a cached embedding.
/// IMPORTANT: no raw `text` field — allow_world_model_raw_text_storage = false means
/// raw text is never persisted to disk (DEC-JEVAL-04).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EmbeddingCacheRecord {
    key: String,
    vector: EmbeddingVector,
    created_at: DateTime<Utc>,
    /// sha256 of the exact text post-redaction — used for cache validation, not re-embedding.
    text_hash: String,
    /// Schema version at write time — stored for provenance; the key already encodes this.
    eval_schema_version: u32,
    /// Source hash at write time — provenance only, never hashed into the cache key.
    source_hash: String,
}

impl CachedEmbeddingAdapter {
    pub fn new(
        inner: Box<dyn WorldEmbeddingAdapter>,
        config: EmbeddingCacheConfig,
        allow_cache: bool,
    ) -> Self {
        Self {
            inner,
            config,
            allow_cache,
        }
    }

    fn cache_path(&self, key: &str) -> PathBuf {
        self.config
            .cache_dir
            .join(&key[0..2])
            .join(format!("{key}.json"))
    }

    fn cache_key(&self, request: &EmbeddingRequest) -> String {
        // Hash the exact post-redaction text first so the outer key never encodes raw text.
        let text_hash = {
            let mut h = Sha256::new();
            h.update(request.text.as_bytes());
            hex::encode(h.finalize())
        };

        let mut hasher = Sha256::new();
        hasher.update(self.inner.provider_name().as_bytes());
        hasher.update(b"\0");
        hasher.update(self.inner.model_name().as_bytes());
        hasher.update(b"\0");
        hasher.update(self.inner.dimensions().to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(request.redaction_policy.as_bytes());
        hasher.update(b"\0");
        // eval_schema_version replaces source_hash in the key: bumping the version invalidates
        // all cached entries. source_hash was invariant to row-content changes when row_ids were
        // unchanged, causing silent stale cache hits.
        hasher.update(self.config.eval_schema_version.to_string().as_bytes());
        hasher.update(b"\0");
        // text_hash (sha256 of the exact post-redaction text) — sensitive to text changes.
        hasher.update(text_hash.as_bytes());
        hex::encode(hasher.finalize())
    }

    fn read_cached(&self, key: &str) -> Result<Option<EmbeddingVector>> {
        if !self.config.cache_enabled {
            return Ok(None);
        }
        let path = self.cache_path(key);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(path)?;
        let record: EmbeddingCacheRecord = serde_json::from_str(&content)?;
        Ok((record.key == key).then_some(record.vector))
    }

    fn write_cached(
        &self,
        key: &str,
        request: &EmbeddingRequest,
        vector: &EmbeddingVector,
    ) -> Result<()> {
        if !self.config.cache_enabled {
            return Ok(());
        }
        // Policy gate: resolved at construction. When false, skip persistence (read-through only).
        if !self.allow_cache {
            return Ok(());
        }
        let text_hash = {
            let mut h = Sha256::new();
            h.update(request.text.as_bytes());
            hex::encode(h.finalize())
        };
        let path = self.cache_path(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let record = EmbeddingCacheRecord {
            key: key.to_string(),
            vector: vector.clone(),
            created_at: Utc::now(),
            text_hash,
            eval_schema_version: self.config.eval_schema_version,
            source_hash: request.source_hash.clone(),
        };
        std::fs::write(path, serde_json::to_vec_pretty(&record)?)?;
        // Reuse existing prune_cache — do NOT add a duplicate prune_if_needed() function.
        prune_cache(&self.config.cache_dir, self.config.cache_max_bytes)?;
        Ok(())
    }
}

impl WorldEmbeddingAdapter for CachedEmbeddingAdapter {
    fn backend_kind(&self) -> EmbeddingBackendKind {
        self.inner.backend_kind()
    }

    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    fn provider_name(&self) -> &str {
        self.inner.provider_name()
    }

    fn model_name(&self) -> &str {
        self.inner.model_name()
    }

    fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingVector> {
        let mut request = request.clone();
        if self.config.redact_before_embedding {
            request.text = redact_embedding_text(&request.text);
        }
        let key = self.cache_key(&request);
        if let Some(vector) = self.read_cached(&key)? {
            return Ok(vector);
        }
        let vector = self.inner.embed(&request)?;
        self.write_cached(&key, &request, &vector)?;
        Ok(vector)
    }
}

pub fn redact_embedding_text(text: &str) -> String {
    let mut redacted = text.to_string();
    for (pattern, replacement) in [
        (
            r"(?i)\b([a-z0-9._%+-]+)@([a-z0-9.-]+\.[a-z]{2,})\b",
            "[REDACTED_EMAIL]",
        ),
        (
            r#"(?i)\b(api[_-]?key|token|secret|authorization|bearer)\s*[:=]\s*['"]?[^\s'"]+"#,
            "$1=[REDACTED_SECRET]",
        ),
        (
            r"\b(sk-[A-Za-z0-9_-]{16,}|[A-Fa-f0-9]{32,})\b",
            "[REDACTED_SECRET]",
        ),
    ] {
        redacted = Regex::new(pattern)
            .expect("world-model redaction regex should compile")
            .replace_all(&redacted, replacement)
            .to_string();
    }
    redacted
}

fn normalize(values: &mut [f32]) {
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in values {
            *value /= norm;
        }
    }
}

fn project_vector(values: &[f32], projection_dimensions: usize) -> Vec<f32> {
    if values.len() == projection_dimensions {
        return values.to_vec();
    }

    let mut projected = vec![0.0; projection_dimensions];
    if projection_dimensions == 0 {
        return projected;
    }
    for (idx, value) in values.iter().enumerate() {
        projected[idx % projection_dimensions] += *value;
    }
    normalize(&mut projected);
    projected
}

fn prune_cache(root: &Path, max_bytes: u64) -> Result<()> {
    if max_bytes == 0 || !root.exists() {
        return Ok(());
    }
    let mut files = cache_files(root)?;
    let mut total = files.iter().map(|entry| entry.size).sum::<u64>();
    if total <= max_bytes {
        return Ok(());
    }
    files.sort_by_key(|entry| entry.modified);
    for entry in files {
        if total <= max_bytes {
            break;
        }
        let size = entry.size;
        let _ = std::fs::remove_file(&entry.path);
        total = total.saturating_sub(size);
    }
    Ok(())
}

#[derive(Debug)]
struct CacheFile {
    path: PathBuf,
    size: u64,
    modified: std::time::SystemTime,
}

fn cache_files(root: &Path) -> Result<Vec<CacheFile>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(cache_files(&path)?);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            let meta = entry.metadata()?;
            files.push(CacheFile {
                path,
                size: meta.len(),
                modified: meta.modified().unwrap_or(std::time::UNIX_EPOCH),
            });
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingAdapter {
        calls: Arc<AtomicUsize>,
    }

    impl WorldEmbeddingAdapter for CountingAdapter {
        fn backend_kind(&self) -> EmbeddingBackendKind {
            EmbeddingBackendKind::Local
        }

        fn dimensions(&self) -> usize {
            2
        }

        fn provider_name(&self) -> &str {
            "counting"
        }

        fn model_name(&self) -> &str {
            "test"
        }

        fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingVector> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(EmbeddingVector {
                values: vec![request.text.len() as f32, 1.0],
                provider: "counting".into(),
                model: "test".into(),
                source_hash: request.source_hash.clone(),
                redaction_policy: request.redaction_policy.clone(),
            })
        }
    }

    #[test]
    fn deterministic_hash_adapter_returns_fixed_dimensions() {
        let adapter = DeterministicHashEmbeddingAdapter::new(8).unwrap();
        let request = EmbeddingRequest {
            text: "verify retry failed".into(),
            source_hash: "hash".into(),
            redaction_policy: "default".into(),
        };

        let first = adapter.embed(&request).unwrap();
        let second = adapter.embed(&request).unwrap();
        assert_eq!(first.values.len(), 8);
        assert_eq!(first.values, second.values);
        assert_eq!(first.provider, "local");
    }

    #[test]
    fn projection_folds_vectors_to_world_model_dimension() {
        let projected = project_vector(&[1.0, 0.0, 1.0, 0.0], 2);
        assert_eq!(projected.len(), 2);
        assert!(projected[0] > projected[1]);
    }

    #[test]
    fn redaction_removes_common_secret_shapes() {
        let text = "email steve@example.com token=sk-live-secretsecretsecretsecret";
        let redacted = redact_embedding_text(text);
        assert!(!redacted.contains("steve@example.com"));
        assert!(!redacted.contains("sk-live"));
        assert!(redacted.contains("[REDACTED_EMAIL]"));
        assert!(redacted.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn cached_adapter_reuses_persisted_vectors() {
        let temp = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let adapter = CachedEmbeddingAdapter::new(
            Box::new(CountingAdapter {
                calls: Arc::clone(&calls),
            }),
            EmbeddingCacheConfig {
                cache_dir: temp.path().join("cache"),
                cache_enabled: true,
                cache_max_bytes: 1024 * 1024,
                redact_before_embedding: true,
                eval_schema_version: 1,
            },
            true,
        );
        let request = EmbeddingRequest {
            text: "token=supersecret value".into(),
            source_hash: "source-1".into(),
            redaction_policy: "default".into(),
        };

        let first = adapter.embed(&request).unwrap();
        let second = adapter.embed(&request).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(first.values, second.values);
        assert_eq!(
            first.values[0],
            redact_embedding_text(&request.text).len() as f32
        );
    }

    #[test]
    fn cache_pruning_removes_old_entries() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("aa")).unwrap();
        let path = temp.path().join("aa").join("a.json");
        std::fs::write(&path, "x".repeat(128)).unwrap();
        prune_cache(temp.path(), 1).unwrap();
        assert!(!path.exists());
    }

    fn make_test_cached_adapter(config: EmbeddingCacheConfig) -> CachedEmbeddingAdapter {
        CachedEmbeddingAdapter::new(
            Box::new(DeterministicHashEmbeddingAdapter::new(4).unwrap()),
            config,
            true,
        )
    }

    fn default_cache_config(cache_dir: std::path::PathBuf) -> EmbeddingCacheConfig {
        EmbeddingCacheConfig {
            cache_dir,
            cache_enabled: true,
            cache_max_bytes: 1024 * 1024,
            redact_before_embedding: false,
            eval_schema_version: 1,
        }
    }

    fn default_request() -> EmbeddingRequest {
        EmbeddingRequest {
            text: "hello world".into(),
            source_hash: "hash-a".into(),
            redaction_policy: "none".into(),
        }
    }

    #[test]
    fn cache_key_unchanged_when_only_source_hash_differs() {
        let temp = tempfile::tempdir().unwrap();
        let adapter = make_test_cached_adapter(default_cache_config(temp.path().to_path_buf()));
        let req1 = EmbeddingRequest {
            text: "hello world".into(),
            source_hash: "hash-a".into(),
            redaction_policy: "none".into(),
        };
        let req2 = EmbeddingRequest {
            text: "hello world".into(),
            source_hash: "hash-b".into(),
            redaction_policy: "none".into(),
        };
        assert_eq!(
            adapter.cache_key(&req1),
            adapter.cache_key(&req2),
            "cache_key must not change when only source_hash differs"
        );
    }

    #[test]
    fn cache_key_changes_when_text_changes() {
        let temp = tempfile::tempdir().unwrap();
        let adapter = make_test_cached_adapter(default_cache_config(temp.path().to_path_buf()));
        let req1 = EmbeddingRequest {
            text: "hello".into(),
            ..default_request()
        };
        let req2 = EmbeddingRequest {
            text: "world".into(),
            ..default_request()
        };
        assert_ne!(
            adapter.cache_key(&req1),
            adapter.cache_key(&req2),
            "cache_key must change when text changes"
        );
    }

    #[test]
    fn cache_key_changes_when_eval_schema_version_changes() {
        let temp1 = tempfile::tempdir().unwrap();
        let temp2 = tempfile::tempdir().unwrap();
        let cfg1 = EmbeddingCacheConfig {
            eval_schema_version: 1,
            ..default_cache_config(temp1.path().to_path_buf())
        };
        let cfg2 = EmbeddingCacheConfig {
            eval_schema_version: 2,
            ..default_cache_config(temp2.path().to_path_buf())
        };
        let req = default_request();
        assert_ne!(
            make_test_cached_adapter(cfg1).cache_key(&req),
            make_test_cached_adapter(cfg2).cache_key(&req),
            "cache_key must change when eval_schema_version changes"
        );
    }

    #[test]
    fn allow_cache_false_skips_disk_write() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let calls = Arc::new(AtomicUsize::new(0));
        // Build adapter with allow_cache = false — policy denies persistence.
        let adapter = CachedEmbeddingAdapter::new(
            Box::new(CountingAdapter {
                calls: Arc::clone(&calls),
            }),
            EmbeddingCacheConfig {
                cache_dir: cache_dir.clone(),
                cache_enabled: true,
                cache_max_bytes: 1024 * 1024,
                redact_before_embedding: false,
                eval_schema_version: 1,
            },
            false, // allow_cache = false
        );
        let request = EmbeddingRequest {
            text: "some text".into(),
            source_hash: "s1".into(),
            redaction_policy: "none".into(),
        };
        // embed should succeed (read-through via inner adapter)
        let result = adapter.embed(&request).unwrap();
        assert!(!result.values.is_empty());
        // inner adapter was still called (no cached hit)
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // No file should have been written to cache_dir
        let cache_exists = cache_dir.exists()
            && std::fs::read_dir(&cache_dir)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
        assert!(
            !cache_exists,
            "cache_dir should be empty when allow_cache=false"
        );
    }
}
