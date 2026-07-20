use super::*;
use crate::graph::MemoryGraph;
use crate::types::MemoryType;

#[test]
fn keyword_candidates_use_fts_for_content_title_and_tags() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    let content_id = g
        .store_memory(
            "indexed-content-marker",
            "",
            MemoryType::Fact,
            0.5,
            &[],
            "m",
            "",
        )
        .expect("store failed");
    let title_id = g
        .store_memory(
            "body",
            "indexed-title-marker",
            MemoryType::Fact,
            0.5,
            &[],
            "m",
            "",
        )
        .expect("store failed");
    let tag_id = g
        .store_memory(
            "body",
            "",
            MemoryType::Fact,
            0.5,
            &["indexed-tag-marker".to_string()],
            "m",
            "",
        )
        .expect("store failed");

    for (query, expected_id) in [
        ("indexed-content-marker", content_id),
        ("indexed-title-marker", title_id),
        ("indexed-tag-marker", tag_id),
    ] {
        let candidates = keyword_candidates(g.db(), query, 16).expect("FTS query failed");
        assert!(candidates.used_fts);
        assert!(
            candidates
                .memories
                .iter()
                .any(|memory| memory.id == expected_id)
        );
    }
}

#[test]
fn keyword_fts_tracks_updates_and_deletes() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    let id = g
        .store_memory(
            "before-update-marker",
            "",
            MemoryType::Fact,
            0.5,
            &[],
            "m",
            "",
        )
        .expect("store failed");

    g.update_memory(&id, Some("after-update-marker"), None)
        .expect("update failed");
    let updated =
        keyword_candidates(g.db(), "after-update-marker", 16).expect("updated FTS query failed");
    assert!(updated.used_fts);
    assert!(updated.memories.iter().any(|memory| memory.id == id));

    g.delete_memory(&id).expect("delete failed");
    let deleted =
        keyword_candidates(g.db(), "after-update-marker", 16).expect("deleted FTS query failed");
    assert!(deleted.used_fts);
    assert!(deleted.memories.iter().all(|memory| memory.id != id));
}

#[test]
fn keyword_candidates_fall_back_when_fts_is_unavailable() {
    let db = DbInstance::new("mem", "", "").expect("db creation failed");
    db.run_script(
        ":create memories {
            id: String => content: String, title: String, memory_type: String,
            importance: Float, tags: String, source_type: String, project_path: String,
            created_at: String, updated_at: String, access_count: Int, last_accessed: String
        }",
        Default::default(),
        cozo::ScriptMutability::Mutable,
    )
    .expect("relation creation failed");
    let now = Utc::now().to_rfc3339();
    let params = std::collections::BTreeMap::from([
        ("id".to_string(), cozo::DataValue::from("fallback-id")),
        ("now".to_string(), cozo::DataValue::from(now.as_str())),
    ]);
    db.run_script(
        "?[id, content, title, memory_type, importance, tags, source_type, project_path,
            created_at, updated_at, access_count, last_accessed] <- [[
            $id, 'fallback-marker', '', 'fact', 0.5, '[]', 'test', '', $now, '', 0, ''
        ]] :put memories {id => content, title, memory_type, importance, tags, source_type,
            project_path, created_at, updated_at, access_count, last_accessed}",
        params,
        cozo::ScriptMutability::Mutable,
    )
    .expect("memory insert failed");

    let candidates = keyword_candidates(&db, "fallback-marker", 16).expect("fallback query failed");
    assert!(!candidates.used_fts);
    assert_eq!(candidates.memories.len(), 1);
    assert_eq!(candidates.memories[0].id, "fallback-id");
}

#[test]
fn keyword_candidates_match_any_query_term() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    let rust_id = g
        .store_memory("rust", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    let python_id = g
        .store_memory("python", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");

    let candidates = keyword_candidates(g.db(), "rust python", 16).expect("FTS query failed");
    assert!(
        candidates
            .memories
            .iter()
            .any(|memory| memory.id == rust_id)
    );
    assert!(
        candidates
            .memories
            .iter()
            .any(|memory| memory.id == python_id)
    );
}

#[test]
fn keyword_candidates_support_single_character_terms() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    let id = g
        .store_memory("x", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");

    let candidates = keyword_candidates(g.db(), "x", 16).expect("FTS query failed");
    assert!(candidates.memories.iter().any(|memory| memory.id == id));
}

#[test]
fn recall_candidate_window_keeps_access_boost_winner() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    let winner = g
        .store_memory("needle", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    for _ in 0..7 {
        g.get_memory(&winner).expect("access update failed");
    }
    for i in 0..8 {
        g.store_memory(
            &format!("decoy-{i} {}", "needle ".repeat(32)),
            "",
            MemoryType::Fact,
            0.5,
            &[],
            "m",
            "",
        )
        .expect("store failed");
    }

    let results = g.recall_memories("needle", 1).expect("recall failed");
    assert_eq!(results[0].id, winner);
}

#[test]
fn recall_ranks_access_boost_winner_beyond_fts_window() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    let winner = g
        .store_memory("needle", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    for _ in 0..7 {
        g.get_memory(&winner).expect("access update failed");
    }
    for i in 0..300 {
        g.store_memory(
            &format!("decoy-{i} {}", "needle ".repeat(32)),
            "",
            MemoryType::Fact,
            0.5,
            &[],
            "m",
            "",
        )
        .expect("store failed");
    }

    let results = g.recall_memories("needle", 1).expect("recall failed");
    assert_eq!(results[0].id, winner);
}

#[test]
fn recall_ranks_by_keyword_hits() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    g.store_memory("apple pie", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    g.store_memory(
        "apple pie with apple sauce",
        "",
        MemoryType::Fact,
        0.5,
        &[],
        "m",
        "",
    )
    .expect("store failed");

    let results = g.recall_memories("apple", 10).expect("recall failed");
    assert_eq!(results.len(), 2);
}

#[test]
fn full_scan_contract_warns_only_past_threshold() {
    assert!(full_scan_contract("memory.recall.keyword", 10, Some(5)).is_none());
    let message = full_scan_contract(
        "memory.recall.keyword",
        FULL_SCAN_WARNING_THRESHOLD,
        Some(5),
    )
    .expect("threshold row count should warn");
    assert!(message.contains("full-scan"));
    assert!(message.contains("at most 5"));
}

#[test]
fn recall_respects_limit() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    for i in 0..20 {
        g.store_memory(
            &format!("memory {i} about rust"),
            "",
            MemoryType::Fact,
            0.5,
            &[],
            "m",
            "",
        )
        .expect("store failed");
    }
    let results = g.recall_memories("rust", 5).expect("recall failed");
    assert_eq!(results.len(), 5);
}

#[test]
fn search_with_blank_text_returns_empty_without_scanning() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    g.store_memory("body", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");
    let filter = SearchFilter {
        text: Some("   ".to_string()),
        ..Default::default()
    };

    let candidates =
        structured_search_candidates(g.db(), &filter).expect("candidate search failed");
    assert!(candidates.used_fts);
    assert!(candidates.memories.is_empty());

    let results = search(g.db(), &filter).expect("search failed");

    assert!(results.is_empty());
}

#[test]
fn search_with_blank_tag_returns_empty_without_scanning() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    g.store_memory(
        "body",
        "",
        MemoryType::Fact,
        0.5,
        &["kept".to_string()],
        "m",
        "",
    )
    .expect("store failed");
    let filter = SearchFilter {
        tags: vec!["   ".to_string()],
        ..Default::default()
    };

    let candidates =
        structured_search_candidates(g.db(), &filter).expect("candidate search failed");
    assert!(candidates.used_fts);
    assert!(candidates.memories.is_empty());

    let results = search(g.db(), &filter).expect("search failed");

    assert!(results.is_empty());
}

#[test]
fn search_with_tags_uses_fts_candidates() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    let expected_id = g
        .store_memory(
            "body",
            "",
            MemoryType::Decision,
            0.5,
            &["indexed-structured-tag".to_string()],
            "m",
            "",
        )
        .expect("store failed");
    let filter = SearchFilter {
        tags: vec!["indexed-structured-tag".to_string()],
        ..Default::default()
    };

    let candidates =
        structured_search_candidates(g.db(), &filter).expect("candidate search failed");
    assert!(candidates.used_fts);

    let results = search(g.db(), &filter).expect("search failed");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, expected_id);
}

#[test]
fn search_with_text_uses_fts_candidates() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    let expected_id = g
        .store_memory(
            "indexed-search-marker",
            "",
            MemoryType::Decision,
            0.5,
            &["kept".to_string()],
            "m",
            "",
        )
        .expect("store failed");
    let filter = SearchFilter {
        memory_type: Some(MemoryType::Decision),
        tags: vec!["kept".to_string()],
        text: Some("indexed-search-marker".to_string()),
        ..Default::default()
    };

    let candidates =
        structured_search_candidates(g.db(), &filter).expect("candidate search failed");
    assert!(candidates.used_fts);

    let results = search(g.db(), &filter).expect("search failed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, expected_id);
}

#[test]
fn search_with_date_range() {
    let g = MemoryGraph::in_memory().expect("graph creation failed");
    g.store_memory("x", "", MemoryType::Fact, 0.5, &[], "m", "")
        .expect("store failed");

    let future = Utc::now() + chrono::Duration::days(1);
    let filter = SearchFilter {
        date_from: Some(future),
        ..Default::default()
    };
    let results = g.search_memories(&filter).expect("search failed");
    assert!(results.is_empty());
}
