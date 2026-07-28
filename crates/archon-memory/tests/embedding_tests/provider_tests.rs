use super::support::MockProvider;
use super::*;
// EmbeddingProvider trait contract
// ---------------------------------------------------------------------------

#[test]
fn provider_trait_embed_returns_correct_count() {
    let provider = MockProvider::new(768);
    let texts = vec!["hello".into(), "world".into(), "foo".into()];
    let vecs = provider.embed(&texts).expect("embed should succeed");
    assert_eq!(vecs.len(), 3);
}

#[test]
fn provider_trait_dimensions_match_vectors() {
    let provider = MockProvider::new(768);
    let texts = vec!["test".into()];
    let vecs = provider.embed(&texts).expect("embed should succeed");
    assert_eq!(vecs[0].len(), provider.dimensions());
}

#[test]
fn provider_trait_empty_input_returns_empty() {
    let provider = MockProvider::new(768);
    let vecs = provider.embed(&[]).expect("embed should succeed");
    assert!(vecs.is_empty());
}

// ---------------------------------------------------------------------------
// EmbeddingConfig + factory
// ---------------------------------------------------------------------------

#[test]
fn config_default_values() {
    let cfg = EmbeddingConfig::default();
    assert_eq!(cfg.provider, EmbeddingProviderKind::Auto);
    assert!((cfg.hybrid_alpha - 0.3).abs() < f32::EPSILON);
}

#[test]
fn factory_creates_local_when_no_api_key() {
    // Ensure OPENAI_API_KEY and ARCHON_MEMORY_OPENAIKEY are NOT set for this test
    // (the factory should fall back to local when keys are absent).
    // We only verify the factory doesn't error; actual model loading is lazy.
    // SAFETY: tests run single-threaded via --test-threads=1 or in isolation.
    unsafe {
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("ARCHON_MEMORY_OPENAIKEY");
    }
    let cfg = EmbeddingConfig {
        provider: EmbeddingProviderKind::Auto,
        hybrid_alpha: 0.3,
    };
    let provider = create_provider(&cfg).expect("factory should succeed for local");
    // Local provider returns 768 dimensions
    assert_eq!(provider.dimensions(), 768);
}

#[test]
fn factory_rejects_openai_without_key() {
    // SAFETY: tests run single-threaded via --test-threads=1 or in isolation.
    unsafe {
        std::env::remove_var("OPENAI_API_KEY");
        std::env::remove_var("ARCHON_MEMORY_OPENAIKEY");
    }
    let cfg = EmbeddingConfig {
        provider: EmbeddingProviderKind::OpenAI,
        hybrid_alpha: 0.3,
    };
    let result = create_provider(&cfg);
    assert!(result.is_err(), "openai provider without key should fail");
}

// ---------------------------------------------------------------------------
