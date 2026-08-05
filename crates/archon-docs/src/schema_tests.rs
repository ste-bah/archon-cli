use super::*;

fn test_db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).unwrap()
}

/// Helper: collect relation names from `::relations` output.
fn relation_names(db: &DbInstance) -> Vec<String> {
    let result = db
        .run_script(
            "::relations",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .expect("::relations must succeed");
    result
        .rows
        .iter()
        .filter_map(|row| row.first().and_then(|v| v.get_str()).map(str::to_string))
        .collect()
}

/// Phase 1 gate: all four ported relations exist after `ensure_doc_schema()`.
#[test]
fn test_phase1_relations_created() {
    let db = test_db();
    ensure_doc_schema(&db).unwrap();
    let names = relation_names(&db);
    for rel in [
        "doc_chunk_sentences",
        "doc_chunk_blocks",
        "doc_chunk_page_breaks",
        "doc_image_ocr_status",
    ] {
        assert!(
            names.contains(&rel.to_string()),
            "ensure_doc_schema must create relation '{rel}'"
        );
    }
}

/// Phase 1 gate: the dual FTS indices are preserved after the new relations are added.
#[test]
fn test_dual_fts_still_present_after_phase1() {
    let db = test_db();
    ensure_doc_schema(&db).unwrap();
    // Query each FTS index — if the index is missing, the query will error.
    let fts_check = |index: &str| {
        db.run_script(
            &format!("?[chunk_id, content] := *doc_chunks{{chunk_id, content}}, ~doc_chunks:{index}{{chunk_id | query: \"\", k: 1}}"),
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
    };
    // An empty query is always valid even on an empty table; what matters is no "index not found" error.
    // We specifically check that the error (if any) is NOT "index not found".
    for index in ["chunk_content_fts", "chunk_exact_fts"] {
        let result = fts_check(index);
        if let Err(ref e) = result {
            let msg = e.to_string();
            assert!(
                !msg.to_lowercase().contains("not found")
                    && !msg.to_lowercase().contains("no such"),
                "FTS index '{index}' missing after ensure_doc_schema: {msg}"
            );
        }
    }
}

#[test]
fn test_ensure_schema_idempotent() {
    let db = test_db();
    ensure_doc_schema(&db).unwrap();
    ensure_doc_schema(&db).unwrap(); // second call must not panic
}

#[test]
fn test_run_create_propagates_real_errors() {
    let db = test_db();
    // Syntax error should propagate, not be silently swallowed
    let result = run_create(&db, ":create bad_syntax {");
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(!msg.contains("conflicts with an existing"));
    assert!(!msg.contains("already exists"));
}

#[test]
fn test_run_create_ignores_both_already_exists_phrasings() {
    let db = test_db();
    // Create once — succeeds
    run_create(&db, ":create phrasing_test { id: String => val: String }").unwrap();
    // Create again — fails with an "already exists" error, must be ignored
    run_create(&db, ":create phrasing_test { id: String => val: String }").unwrap();
    // If we got here, the error was correctly suppressed
}

#[test]
fn test_vec_schema_idempotent() {
    let db = test_db();
    ensure_doc_schema(&db).unwrap();
    ensure_vec_schema(&db, 768, None).unwrap();
    // Second call must be silent
    ensure_vec_schema(&db, 768, None).unwrap();
}

#[test]
fn test_ensure_vec_schema_creates_relation_lazily() {
    let db = test_db();
    // Fresh DB: only doc schema (NOT vec schema)
    ensure_doc_schema(&db).unwrap();

    // Inserting a vector must fail because vec_text_chunks doesn't exist
    let mut params = std::collections::BTreeMap::new();
    let v = ndarray::Array1::from_vec(vec![0.0_f32; 768]);
    params.insert("cid".to_string(), cozo::DataValue::from("test-chunk"));
    params.insert(
        "emb".to_string(),
        cozo::DataValue::Vec(cozo::Vector::F32(v)),
    );
    params.insert("prov".to_string(), cozo::DataValue::from("test"));
    let before = db.run_script(
        "?[chunk_id, embedding, provider] <- [[$cid, $emb, $prov]]
             :put vec_text_chunks { chunk_id => embedding, provider }",
        params.clone(),
        cozo::ScriptMutability::Mutable,
    );
    assert!(
        before.is_err(),
        "vector insert must fail before ensure_vec_schema"
    );

    // Now create vec schema
    ensure_vec_schema(&db, 768, None).unwrap();

    // Same insert must succeed after ensure_vec_schema
    let after = db.run_script(
        "?[chunk_id, embedding, provider] <- [[$cid, $emb, $prov]]
             :put vec_text_chunks { chunk_id => embedding, provider }",
        params,
        cozo::ScriptMutability::Mutable,
    );
    assert!(
        after.is_ok(),
        "vector insert must succeed after ensure_vec_schema"
    );
}

#[test]
fn test_vec_page_images_migrates_on_dim_change() {
    let db = test_db();
    // Simulate a pre-CLIP DB: vec_page_images sized to the TEXT dim (768).
    ensure_vec_page_images(&db, 768).unwrap();
    assert_eq!(existing_vec_page_images_dim(&db), Some(768));

    let mut params = std::collections::BTreeMap::new();
    let v512 = ndarray::Array1::from_vec(vec![0.0_f32; 512]);
    params.insert("pid".to_string(), cozo::DataValue::from("page-doc-1"));
    params.insert(
        "emb".to_string(),
        cozo::DataValue::Vec(cozo::Vector::F32(v512)),
    );
    params.insert("prov".to_string(), cozo::DataValue::from("clip"));
    let put = "?[page_id, embedding, provider] <- [[$pid, $emb, $prov]]
             :put vec_page_images { page_id => embedding, provider }";

    // A 512-dim CLIP vector must NOT fit the stale 768-dim relation yet (the bug).
    let before = db.run_script(put, params.clone(), cozo::ScriptMutability::Mutable);
    assert!(
        before.is_err(),
        "512-dim insert must fail against a stale 768-dim relation"
    );

    // Re-ensure at the CLIP image dim → migrate (drop + recreate at 512).
    ensure_vec_page_images(&db, 512).unwrap();
    assert_eq!(existing_vec_page_images_dim(&db), Some(512));

    // The same 512-dim insert must now succeed (fix verified).
    let after = db.run_script(put, params, cozo::ScriptMutability::Mutable);
    assert!(
        after.is_ok(),
        "512-dim insert must succeed after migration: {after:?}"
    );
}

#[test]
fn test_vec_schema_rejects_wrong_dimension() {
    let db = test_db();
    ensure_doc_schema(&db).unwrap();
    ensure_vec_schema(&db, 768, None).unwrap();
    // Insert with wrong dimension must fail
    let mut params = std::collections::BTreeMap::new();
    let wrong_vec = ndarray::Array1::from_vec(vec![0.0_f32; 384]);
    params.insert("cid".to_string(), cozo::DataValue::from("test-chunk"));
    params.insert(
        "emb".to_string(),
        cozo::DataValue::Vec(cozo::Vector::F32(wrong_vec)),
    );
    params.insert("prov".to_string(), cozo::DataValue::from("test"));
    let result = db.run_script(
        "?[chunk_id, embedding, provider] <- [[$cid, $emb, $prov]]
             :put vec_text_chunks { chunk_id => embedding, provider }",
        params,
        cozo::ScriptMutability::Mutable,
    );
    assert!(result.is_err(), "wrong-dimension vector insert must fail");
}

#[test]
fn test_cozo_relation_not_found_marker() {
    let db = test_db();
    // Query a relation that doesn't exist — Cozo must return an error
    // containing COZO_RELATION_NOT_FOUND. If this fails, the Cozo version
    // changed its error phrasing and the constant needs updating.
    let result = db.run_script(
        "?[chunk_id] := *nonexistent_relation_xyz{chunk_id}",
        Default::default(),
        cozo::ScriptMutability::Immutable,
    );
    assert!(result.is_err(), "querying nonexistent relation must fail");
    let msg = format!("{}", result.unwrap_err());
    assert!(
        msg.contains(crate::errors::COZO_RELATION_NOT_FOUND),
        "Cozo error must contain COZO_RELATION_NOT_FOUND marker.\n\
             Expected to contain: {}\n\
             Actual error: {msg}",
        crate::errors::COZO_RELATION_NOT_FOUND,
    );
}
