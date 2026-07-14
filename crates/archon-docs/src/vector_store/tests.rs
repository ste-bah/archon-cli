use super::*;

#[test]
fn count_vectors_does_not_decode_vector_payloads() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    store
        .db
        .put(vector_key("test", "broken"), b"not-a-vector")
        .unwrap();

    assert_eq!(store.count_vectors(Some("test")).unwrap(), 1);
}

#[test]
fn count_vectors_respects_provider_prefixes() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    store
        .put_vectors(&[
            VectorWrite {
                chunk_id: "a-1",
                content_hash: "hash-a-1",
                provider: "provider-a",
                embedding: &[1.0, 0.0],
            },
            VectorWrite {
                chunk_id: "a-2",
                content_hash: "hash-a-2",
                provider: "provider-a",
                embedding: &[0.9, 0.1],
            },
            VectorWrite {
                chunk_id: "b-1",
                content_hash: "hash-b-1",
                provider: "provider-b",
                embedding: &[0.0, 1.0],
            },
        ])
        .unwrap();

    assert_eq!(store.count_vectors(Some("provider-a")).unwrap(), 2);
    assert_eq!(store.count_vectors(Some("provider-b")).unwrap(), 1);
    assert_eq!(store.count_vectors(Some("missing")).unwrap(), 0);
    assert_eq!(store.count_vectors(None).unwrap(), 3);
}

#[test]
fn rocksdb_store_round_trips_vectors_and_cache() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    let rows = [VectorWrite {
        chunk_id: "chunk-a",
        content_hash: "hash-a",
        provider: "test",
        embedding: &[0.25, 0.75],
    }];
    assert_eq!(store.put_vectors(&rows).unwrap(), 1);
    assert_eq!(store.stats(Some("test")).unwrap().raw_vectors, 1);
    assert_eq!(
        store.cached_embedding("test", "hash-a").unwrap().unwrap(),
        vec![0.25, 0.75]
    );
}

#[test]
fn rust_hnsw_search_returns_nearest_chunk() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    let rows = [
        VectorWrite {
            chunk_id: "chunk-a",
            content_hash: "hash-a",
            provider: "test",
            embedding: &[1.0, 0.0],
        },
        VectorWrite {
            chunk_id: "chunk-b",
            content_hash: "hash-b",
            provider: "test",
            embedding: &[0.0, 1.0],
        },
    ];
    store.put_vectors(&rows).unwrap();
    let hits = store
        .search_in_memory("test", &[0.99, 0.01], 1, 16, None)
        .unwrap();
    assert_eq!(hits[0].chunk_id, "chunk-a");
}

#[test]
fn rust_hnsw_search_uses_one_identifier_probe_per_hit() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    let owned: Vec<_> = (0..64)
        .map(|index| {
            (
                format!("chunk-{index}"),
                format!("hash-{index}"),
                vec![1.0, index as f32 / 64.0],
            )
        })
        .collect();
    let rows: Vec<_> = owned
        .iter()
        .map(|(chunk_id, content_hash, embedding)| VectorWrite {
            chunk_id,
            content_hash,
            provider: "test",
            embedding,
        })
        .collect();
    store.put_vectors(&rows).unwrap();
    HIT_RESOLUTION_PROBES.with(|probes| probes.set(0));

    let hits = store
        .search_in_memory("test", &[1.0, 0.0], 8, 16, None)
        .unwrap();

    assert_eq!(hits.len(), 8);
    HIT_RESOLUTION_PROBES.with(|probes| assert_eq!(probes.get(), hits.len()));
}
#[test]
fn chunk_ids_by_hnsw_id_preserves_first_chunk_id() {
    let records = [
        RawVectorRecord {
            chunk_id: "first".into(),
            provider: "test".into(),
            vector: vec![1.0],
            hnsw_id: 7,
        },
        RawVectorRecord {
            chunk_id: "second".into(),
            provider: "test".into(),
            vector: vec![0.0],
            hnsw_id: 7,
        },
    ];

    let chunk_ids = chunk_ids_by_hnsw_id(&records);

    assert_eq!(chunk_ids.get(&7).map(String::as_str), Some("first"));
}
#[test]
fn put_vectors_reports_only_non_empty_embeddings() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    let rows = [
        VectorWrite {
            chunk_id: "chunk-valid",
            content_hash: "hash-valid",
            provider: "test",
            embedding: &[1.0, 0.0],
        },
        VectorWrite {
            chunk_id: "chunk-empty",
            content_hash: "hash-empty",
            provider: "test",
            embedding: &[],
        },
    ];

    assert_eq!(store.put_vectors(&rows).unwrap(), 1);
    assert_eq!(store.stats(Some("test")).unwrap().raw_vectors, 1);
    assert!(store.has_vector("test", "chunk-valid").unwrap());
    assert!(!store.has_vector("test", "chunk-empty").unwrap());
}

#[test]
fn put_vectors_all_empty_reports_zero_and_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    let rows = [VectorWrite {
        chunk_id: "chunk-empty",
        content_hash: "hash-empty",
        provider: "test",
        embedding: &[],
    }];

    assert_eq!(store.put_vectors(&rows).unwrap(), 0);
    assert_eq!(store.stats(Some("test")).unwrap().raw_vectors, 0);
    assert_eq!(store.stats(Some("test")).unwrap().cache_entries, 0);
}

#[test]
fn put_vectors_empty_input_reports_zero() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();

    assert_eq!(store.put_vectors(&[]).unwrap(), 0);
    assert_eq!(store.stats(None).unwrap().raw_vectors, 0);
}

#[test]
fn put_vectors_duplicate_keys_report_unique_count_and_keep_last_payload() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    let rows = [
        VectorWrite {
            chunk_id: "chunk-duplicate",
            content_hash: "hash-first",
            provider: "test",
            embedding: &[1.0, 0.0],
        },
        VectorWrite {
            chunk_id: "chunk-duplicate",
            content_hash: "hash-last",
            provider: "test",
            embedding: &[0.0, 1.0],
        },
    ];

    assert_eq!(store.put_vectors(&rows).unwrap(), 1);
    assert_eq!(store.count_vectors(Some("test")).unwrap(), 1);
    let raw_vector = store
        .db
        .get(vector_key("test", "chunk-duplicate"))
        .unwrap()
        .unwrap();
    assert_eq!(decode_vector(&raw_vector).unwrap(), vec![0.0, 1.0]);
}

#[test]
fn put_vectors_mixed_duplicate_unique_and_empty_rows_report_exact_cardinality() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    let rows = [
        VectorWrite {
            chunk_id: "chunk-duplicate",
            content_hash: "hash-first",
            provider: "test",
            embedding: &[1.0, 0.0],
        },
        VectorWrite {
            chunk_id: "chunk-duplicate",
            content_hash: "hash-last",
            provider: "test",
            embedding: &[0.0, 1.0],
        },
        VectorWrite {
            chunk_id: "chunk-unique",
            content_hash: "hash-unique",
            provider: "test",
            embedding: &[0.5, 0.5],
        },
        VectorWrite {
            chunk_id: "chunk-empty",
            content_hash: "hash-empty",
            provider: "test",
            embedding: &[],
        },
    ];

    assert_eq!(store.put_vectors(&rows).unwrap(), 2);
    assert_eq!(store.count_vectors(Some("test")).unwrap(), 2);
}

#[test]
fn invalid_providers_fail_before_writes_and_do_not_leak_into_valid_counts() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    let rows = [
        VectorWrite {
            chunk_id: "chunk-valid",
            content_hash: "hash-valid",
            provider: "valid",
            embedding: &[1.0, 0.0],
        },
        VectorWrite {
            chunk_id: "chunk-invalid",
            content_hash: "hash-invalid",
            provider: "valid/invalid",
            embedding: &[0.0, 1.0],
        },
    ];

    assert!(store.put_vectors(&rows).is_err());
    assert_eq!(store.count_vectors(Some("valid")).unwrap(), 0);
    assert!(
        store
            .put_vectors(&[VectorWrite {
                chunk_id: "chunk-empty-provider",
                content_hash: "hash-empty-provider",
                provider: "",
                embedding: &[1.0, 0.0],
            }])
            .is_err()
    );
    assert_eq!(store.count_vectors(None).unwrap(), 0);

    for provider in ["", "/", "valid/invalid"] {
        assert!(store.has_vector(provider, "chunk").is_err());
        assert!(store.cached_embedding(provider, "hash").is_err());
        assert!(store.count_vectors(Some(provider)).is_err());
        assert!(store.stats(Some(provider)).is_err());
        assert!(store.build_hnsw(provider, 2, None).is_err());
        assert!(
            store
                .search_in_memory(provider, &[1.0, 0.0], 1, 16, None)
                .is_err()
        );
        assert!(store.latest_hnsw_manifest(provider).is_err());
    }
}

#[test]
fn safe_provider_encodes_distinct_punctuation_injectively() {
    assert_ne!(safe_provider("a.b"), safe_provider("a_b"));
}

#[test]
fn safe_provider_escapes_the_sentinel_prefix() {
    assert_ne!(safe_provider("~"), safe_provider("~7e"));
    assert_eq!(safe_provider("~"), "~~");
    assert_eq!(safe_provider("~7e"), "~~7e");
    assert_eq!(safe_provider("."), "~2e");
    assert_eq!(safe_provider("fastembed-onnx"), "fastembed-onnx");
}

#[test]
fn safe_provider_preserves_historical_hyphenated_names() {
    assert_eq!(safe_provider("fastembed-onnx"), "fastembed-onnx");
}

#[test]
fn latest_hnsw_manifest_rejects_provider_mismatch() {
    let temp = tempfile::tempdir().unwrap();
    let store = DocVectorStore::open(temp.path()).unwrap();
    let requested_provider = "requested";
    let manifest_path = store.hnsw_manifest_path(requested_provider);
    std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
    let manifest = HnswManifest {
        provider: "other".into(),
        dimension: 2,
        vector_count: 1,
        dump_basename: "index".into(),
        created_at: "2026-07-13T00:00:00Z".into(),
    };
    std::fs::write(manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let error = store.latest_hnsw_manifest(requested_provider).unwrap_err();

    assert!(error.to_string().contains("provider mismatch"));
}
