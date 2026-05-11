use anyhow::{Result, bail};

use archon_world_model::embedding::{
    CachedEmbeddingAdapter, DeterministicHashEmbeddingAdapter, EmbeddingBackendKind,
    EmbeddingCacheConfig, MemoryEmbeddingAdapter, WorldEmbeddingAdapter,
};
use archon_world_model::embedding_audit::AuditedEmbeddingAdapter;

pub(super) fn build_embedding_adapter(
    config: &archon_core::config::ArchonConfig,
) -> Result<Box<dyn WorldEmbeddingAdapter>> {
    let embeddings = &config.learning.world_model.embeddings;
    let projection_dim = config.learning.world_model.state_dim;
    let source = embeddings.source.as_str();
    let provider = embeddings.provider.as_str();
    let policy =
        archon_policy::load_effective_policy(&std::env::current_dir()?).unwrap_or_default();
    let external_decision = policy.world_model_third_party_embeddings_decision();
    let third_party_allowed = embeddings.allow_third_party && external_decision.allowed;

    if provider == "deterministic-hash" {
        return Ok(Box::new(DeterministicHashEmbeddingAdapter::new(
            projection_dim,
        )?));
    }

    let adapter: Box<dyn WorldEmbeddingAdapter> = match source {
        "local" | "auto" if provider == "fastembed" => {
            Box::new(MemoryEmbeddingAdapter::local_fastembed(projection_dim)?)
        }
        "third_party" if provider == "openai" && third_party_allowed => {
            Box::new(MemoryEmbeddingAdapter::openai(
                projection_dim,
                true,
                non_empty(&embeddings.external_api_key_env),
            )?)
        }
        "third_party" if provider == "openai" => bail!("{}", external_decision.reason),
        "auto" if third_party_allowed && provider == "openai" => {
            Box::new(MemoryEmbeddingAdapter::openai(
                projection_dim,
                true,
                non_empty(&embeddings.external_api_key_env),
            )?)
        }
        "auto" if provider == "openai" => {
            Box::new(MemoryEmbeddingAdapter::local_fastembed(projection_dim)?)
        }
        _ => bail!(
            "unsupported world-model embedding source/provider: source={} provider={}",
            embeddings.source,
            embeddings.provider
        ),
    };
    let adapter = if adapter.backend_kind() == EmbeddingBackendKind::External {
        Box::new(AuditedEmbeddingAdapter::new(
            adapter,
            super::world_model_root()?
                .join("ledgers")
                .join("embedding-policy-events.jsonl"),
            external_decision.reason,
        )) as Box<dyn WorldEmbeddingAdapter>
    } else {
        adapter
    };

    if embeddings.cache_enabled || embeddings.redact_before_embedding {
        Ok(Box::new(CachedEmbeddingAdapter::new(
            adapter,
            EmbeddingCacheConfig {
                cache_dir: super::world_model_root()?.join("embeddings").join("cache"),
                cache_enabled: embeddings.cache_enabled,
                cache_max_bytes: embeddings.cache_max_mb.saturating_mul(1024 * 1024),
                redact_before_embedding: embeddings.redact_before_embedding,
            },
        )))
    } else {
        Ok(adapter)
    }
}

fn non_empty(value: &str) -> Option<&str> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}
