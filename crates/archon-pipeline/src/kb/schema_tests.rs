use super::*;

#[test]
fn interrupted_swap_before_config_restores_previous_embedding_space() {
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    ensure_kb_embedding_schema(&db, "provider-a", 2, None).unwrap();
    db.run_script(
        "?[node_id, embedding] <- [['old-node', vec([1.0, 0.0])]] \
         :put kb_embeddings { node_id => embedding }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    store_embedding_migration(&db, "provider-b", 3).unwrap();
    run_create(&db, &embedding_relation_script("kb_embeddings_staging", 3)).unwrap();
    db.run_script(
        "?[node_id, embedding] <- [['new-node', vec([0.0, 1.0, 0.0])]] \
         :put kb_embeddings_staging { node_id => embedding }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    drop_embedding_indices(&db, "kb_embeddings").unwrap();
    db.run_script(
        "::rename kb_embeddings -> kb_embeddings_backup, \
         kb_embeddings_staging -> kb_embeddings",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    ensure_kb_embedding_schema(&db, "provider-a", 2, None).unwrap();

    let vectors = db
        .run_script(
            "?[node_id] := *kb_embeddings{node_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(vectors.rows.len(), 1);
    assert_eq!(vectors.rows[0][0].get_str(), Some("old-node"));
}

#[test]
fn failed_embedding_migration_preserves_previous_storage_and_config() {
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    ensure_kb_embedding_schema(&db, "provider-a", 2, None).unwrap();
    db.run_script(
        "?[node_id, embedding] <- [['node-a', vec([1.0, 0.0])]] \
         :put kb_embeddings { node_id => embedding }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    db.run_script(
        "::access_level read_only kb_embedding_config",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    assert!(ensure_kb_embedding_schema(&db, "provider-b", 3, None).is_err());

    let vectors = db
        .run_script(
            "?[node_id] := *kb_embeddings{node_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(vectors.rows[0][0].get_str(), Some("node-a"));
    let config = db
        .run_script(
            "?[provider, dimension] := *kb_embedding_config{config_key, provider, dimension}, \
             config_key = 'active'",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(config.rows[0][0].get_str(), Some("provider-a"));
    assert_eq!(config.rows[0][1].get_int(), Some(2));
}
