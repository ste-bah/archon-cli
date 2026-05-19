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

    /// Embed a batch of requests, returning one vector per request in order.
    ///
    /// Default: sequential loop over [`Self::embed`]. Override on concrete types that support
    /// batch-native embedding (e.g. fastembed) to avoid redundant model-load overhead.
    fn embed_batch(&self, requests: &[EmbeddingRequest]) -> Result<Vec<EmbeddingVector>> {
        requests.iter().map(|r| self.embed(r)).collect()
    }
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

    /// Batch-native override: passes all texts to the provider in a single call,
    /// avoiding repeated model initialisation / inference round-trips.
    fn embed_batch(&self, requests: &[EmbeddingRequest]) -> Result<Vec<EmbeddingVector>> {
        let texts: Vec<String> = requests.iter().map(|r| r.text.clone()).collect();
        let all_vectors = self
            .provider
            .embed(&texts)
            .map_err(|error| anyhow::anyhow!("world-model batch embedding failed: {error}"))?;
        if all_vectors.len() != requests.len() {
            anyhow::bail!(
                "embedding provider returned {} vectors for {} requests",
                all_vectors.len(),
                requests.len()
            );
        }
        Ok(all_vectors
            .into_iter()
            .zip(requests.iter())
            .map(|(raw, request)| EmbeddingVector {
                values: project_vector(&raw, self.projection_dimensions),
                provider: self.provider_name.clone(),
                model: self.model_name.clone(),
                source_hash: request.source_hash.clone(),
                redaction_policy: request.redaction_policy.clone(),
            })
            .collect())
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
