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

    // ---------------------------------------------------------------------------
    // Batch tests
    // ---------------------------------------------------------------------------

    /// An inner adapter that overrides `embed_batch` and counts how many times
    /// the batch entry-point is called (not individual `embed` calls).
    struct BatchCountingAdapter {
        batch_calls: Arc<AtomicUsize>,
    }

    impl WorldEmbeddingAdapter for BatchCountingAdapter {
        fn backend_kind(&self) -> EmbeddingBackendKind {
            EmbeddingBackendKind::Local
        }
        fn dimensions(&self) -> usize {
            4
        }
        fn provider_name(&self) -> &str {
            "batch-counting"
        }
        fn model_name(&self) -> &str {
            "test"
        }
        fn embed(&self, request: &EmbeddingRequest) -> Result<EmbeddingVector> {
            // Delegate to a fresh deterministic adapter so singular calls still work.
            DeterministicHashEmbeddingAdapter::new(4)
                .unwrap()
                .embed(request)
        }
        fn embed_batch(&self, requests: &[EmbeddingRequest]) -> Result<Vec<EmbeddingVector>> {
            self.batch_calls.fetch_add(1, Ordering::SeqCst);
            // Use the real deterministic adapter for output correctness.
            let inner = DeterministicHashEmbeddingAdapter::new(4).unwrap();
            requests.iter().map(|r| inner.embed(r)).collect()
        }
    }

    fn make_batch_requests(texts: &[&str]) -> Vec<EmbeddingRequest> {
        texts
            .iter()
            .map(|t| EmbeddingRequest {
                text: (*t).to_string(),
                source_hash: format!("h:{t}"),
                redaction_policy: "none".into(),
            })
            .collect()
    }

    #[test]
    fn embed_batch_default_matches_sequential() {
        // DeterministicHashEmbeddingAdapter does NOT override embed_batch,
        // so the default loop is exercised.
        let adapter = DeterministicHashEmbeddingAdapter::new(8).unwrap();
        let requests = make_batch_requests(&["alpha", "beta", "gamma"]);

        let batch = adapter.embed_batch(&requests).unwrap();
        let seq: Vec<EmbeddingVector> =
            requests.iter().map(|r| adapter.embed(r).unwrap()).collect();

        assert_eq!(batch.len(), seq.len());
        for (b, s) in batch.iter().zip(seq.iter()) {
            assert_eq!(b.values, s.values, "batch and sequential must agree");
        }
    }

    #[test]
    fn cached_embed_batch_single_inner_call_for_misses() {
        let temp = tempfile::tempdir().unwrap();
        let batch_calls = Arc::new(AtomicUsize::new(0));
        let adapter = CachedEmbeddingAdapter::new(
            Box::new(BatchCountingAdapter {
                batch_calls: Arc::clone(&batch_calls),
            }),
            EmbeddingCacheConfig {
                cache_dir: temp.path().join("cache"),
                cache_enabled: true,
                cache_max_bytes: 1024 * 1024,
                redact_before_embedding: false,
                eval_schema_version: 1,
            },
            true,
        );
        let requests = make_batch_requests(&["a", "b", "c"]);

        // All three are cache misses — inner.embed_batch must be called exactly once.
        let result = adapter.embed_batch(&requests).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(
            batch_calls.load(Ordering::SeqCst),
            1,
            "CachedEmbeddingAdapter must call inner.embed_batch exactly once for all misses"
        );
    }

    #[test]
    fn cached_embed_batch_hits_skip_inner() {
        let temp = tempfile::tempdir().unwrap();
        let batch_calls = Arc::new(AtomicUsize::new(0));
        let adapter = CachedEmbeddingAdapter::new(
            Box::new(BatchCountingAdapter {
                batch_calls: Arc::clone(&batch_calls),
            }),
            EmbeddingCacheConfig {
                cache_dir: temp.path().join("cache"),
                cache_enabled: true,
                cache_max_bytes: 1024 * 1024,
                redact_before_embedding: false,
                eval_schema_version: 1,
            },
            true,
        );
        let requests = make_batch_requests(&["x", "y"]);

        // First call: 2 misses → 1 inner batch call.
        let first = adapter.embed_batch(&requests).unwrap();
        assert_eq!(batch_calls.load(Ordering::SeqCst), 1);

        // Second call: 2 hits → 0 inner batch calls (total stays at 1).
        let second = adapter.embed_batch(&requests).unwrap();
        assert_eq!(
            batch_calls.load(Ordering::SeqCst),
            1,
            "cache hits must not trigger additional inner batch calls"
        );
        assert_eq!(first[0].values, second[0].values);
        assert_eq!(first[1].values, second[1].values);
    }

    #[test]
    fn cached_embed_batch_partial_hits() {
        let temp = tempfile::tempdir().unwrap();
        let batch_calls = Arc::new(AtomicUsize::new(0));
        let adapter = CachedEmbeddingAdapter::new(
            Box::new(BatchCountingAdapter {
                batch_calls: Arc::clone(&batch_calls),
            }),
            EmbeddingCacheConfig {
                cache_dir: temp.path().join("cache"),
                cache_enabled: true,
                cache_max_bytes: 1024 * 1024,
                redact_before_embedding: false,
                eval_schema_version: 1,
            },
            true,
        );

        // Prime cache for "m" only.
        let prime = make_batch_requests(&["m"]);
        adapter.embed_batch(&prime).unwrap();
        let calls_after_prime = batch_calls.load(Ordering::SeqCst);

        // Now batch with 1 hit ("m") and 1 miss ("n").
        let requests = make_batch_requests(&["m", "n"]);
        let result = adapter.embed_batch(&requests).unwrap();
        assert_eq!(result.len(), 2);
        // One additional inner batch call for the single miss.
        assert_eq!(
            batch_calls.load(Ordering::SeqCst),
            calls_after_prime + 1,
            "exactly one inner batch call for the one miss"
        );
    }
}
