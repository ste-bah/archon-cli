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

    /// Batched override: single cache-lookup pass, then ONE inner `embed_batch` call for all
    /// misses.  Preserves result order.
    fn embed_batch(&self, requests: &[EmbeddingRequest]) -> Result<Vec<EmbeddingVector>> {
        // Apply redaction and compute cache keys up-front.
        let effective: Vec<EmbeddingRequest> = requests
            .iter()
            .map(|r| {
                if self.config.redact_before_embedding {
                    EmbeddingRequest {
                        text: redact_embedding_text(&r.text),
                        ..r.clone()
                    }
                } else {
                    r.clone()
                }
            })
            .collect();
        let keys: Vec<String> = effective.iter().map(|r| self.cache_key(r)).collect();

        // Cache lookup pass — collect hits, record miss indices.
        let mut results: Vec<Option<EmbeddingVector>> = vec![None; requests.len()];
        let mut miss_indices: Vec<usize> = Vec::new();
        for (i, key) in keys.iter().enumerate() {
            if let Ok(Some(v)) = self.read_cached(key) {
                results[i] = Some(v);
            } else {
                miss_indices.push(i);
            }
        }

        // Single inner embed_batch call for all misses.
        if !miss_indices.is_empty() {
            let miss_requests: Vec<EmbeddingRequest> =
                miss_indices.iter().map(|&i| effective[i].clone()).collect();
            let miss_vectors = self.inner.embed_batch(&miss_requests)?;
            for (slot, &orig_idx) in miss_indices.iter().enumerate() {
                let vector = miss_vectors[slot].clone();
                let _ = self.write_cached(&keys[orig_idx], &effective[orig_idx], &vector);
                results[orig_idx] = Some(vector);
            }
        }

        results
            .into_iter()
            .map(|r| r.ok_or_else(|| anyhow::anyhow!("missing batch result")))
            .collect()
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
