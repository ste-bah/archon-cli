use super::*;
// OpenAI provider: error handling (no HTTP mocking, just unit-level)
// ---------------------------------------------------------------------------

#[test]
fn openai_provider_requires_api_key() {
    use archon_memory::embedding::openai::OpenAIEmbedding;
    // Empty key should fail on construction
    let result = OpenAIEmbedding::new("");
    assert!(result.is_err());
}

/// Regression: v0.1.6–v0.1.11 — calling embed() from within a tokio async
/// runtime panicked with "Cannot start a runtime from within a runtime"
/// because reqwest::blocking::Client::send() internally constructs a tokio
/// runtime. v0.1.12 wraps embed() in tokio::task::block_in_place.
///
/// Uses a fake API key so OpenAI returns 401 immediately (no retry). The
/// test passes if embed() returns an Err (401) without panicking.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn openai_embed_does_not_panic_inside_tokio_runtime() {
    use archon_memory::embedding::EmbeddingProvider;
    use archon_memory::embedding::openai::OpenAIEmbedding;

    let provider = OpenAIEmbedding::new("test-fake-key").expect("construct with fake key");
    // Must NOT panic with "Cannot start a runtime from within a runtime"
    let result = provider.embed(&["test text".into()]);
    // 401 Unauthorized is expected (fake key); the point is no panic
    match result {
        Ok(_) => {} // unlikely but not a failure
        Err(e) => {
            let msg = format!("{e}");
            assert!(
                msg.contains("401") || msg.contains("invalid"),
                "expected 401, got: {msg}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// EmbeddingConfig defaults (archon-core config test lives in archon-core)
// ---------------------------------------------------------------------------

#[test]
fn embedding_config_serde_round_trip() {
    let cfg = EmbeddingConfig {
        provider: EmbeddingProviderKind::Local,
        hybrid_alpha: 0.5,
        ..Default::default()
    };
    let json = serde_json::to_string(&cfg).expect("serialize");
    let parsed: EmbeddingConfig = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed.provider, EmbeddingProviderKind::Local);
    assert!((parsed.hybrid_alpha - 0.5).abs() < f32::EPSILON);
}
