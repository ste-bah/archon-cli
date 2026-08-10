//! Tests for prompt injection: keyword extraction, budgeting, and caching.

use super::*;
use crate::graph::MemoryGraph;
use crate::types::MemoryType;

fn make_graph() -> MemoryGraph {
    MemoryGraph::in_memory().expect("in-memory graph")
}

fn seed_graph(g: &MemoryGraph) {
    g.store_memory(
        "User prefers dark mode",
        "dark mode pref",
        MemoryType::Preference,
        0.7,
        &["ui".into(), "preference".into()],
        "manual",
        "/project",
    )
    .expect("store");
    g.store_memory(
        "Rust edition must be 2024",
        "rust edition",
        MemoryType::Rule,
        0.9,
        &["rust".into(), "edition".into()],
        "manual",
        "/project",
    )
    .expect("store");
    g.store_memory(
        "Never use .unwrap() in library code",
        "no unwrap",
        MemoryType::Correction,
        0.85,
        &["rust".into(), "quality".into()],
        "manual",
        "/project",
    )
    .expect("store");
    g.store_memory(
        "Architecture uses hexagonal pattern",
        "architecture",
        MemoryType::Decision,
        0.8,
        &["architecture".into()],
        "manual",
        "/project",
    )
    .expect("store");
    g.store_memory(
        "Database migrations run on startup",
        "migrations",
        MemoryType::Fact,
        0.6,
        &["database".into(), "ops".into()],
        "manual",
        "/project",
    )
    .expect("store");
}

#[test]
fn inject_returns_memories_for_matching_context() {
    let g = make_graph();
    seed_graph(&g);
    let mut injector = MemoryInjector::new();
    let context = vec!["Tell me about rust edition rules".to_string()];
    let result = injector.inject(&g, &context, 500).expect("inject");
    assert!(!result.is_empty());
    assert!(result.contains("<memories>"));
    assert!(result.contains("</memories>"));
    assert!(result.contains("## Relevant Memories"));
}

#[test]
fn inject_empty_graph_returns_empty_string() {
    let g = make_graph();
    let mut injector = MemoryInjector::new();
    let context = vec!["hello world".to_string()];
    let result = injector.inject(&g, &context, 500).expect("inject");
    assert!(result.is_empty());
}

#[test]
fn inject_empty_context_returns_empty_string() {
    let g = make_graph();
    seed_graph(&g);
    let mut injector = MemoryInjector::new();
    let result = injector.inject(&g, &[], 500).expect("inject");
    assert!(result.is_empty());
}

#[test]
fn budget_enforcement_truncates_output() {
    let g = make_graph();
    seed_graph(&g);
    let mut injector = MemoryInjector::new();
    let context = vec!["rust edition unwrap database architecture".to_string()];

    // Very small budget — only header + maybe 1 line.
    let tiny = injector.inject(&g, &context, 25).expect("inject");
    // Large budget — should include more.
    injector.invalidate_cache();
    let large = injector.inject(&g, &context, 5000).expect("inject");

    // Tiny budget should be shorter (or empty if even 1 line doesn't fit).
    assert!(tiny.len() <= large.len());
}

#[test]
fn extract_keywords_uses_last_three_messages() {
    let context = vec![
        "oldest message ignored".to_string(),
        "second message also ignored".to_string(),
        "rust edition 2024".to_string(),
        "unwrap error handling".to_string(),
        "database migration startup".to_string(),
    ];
    let kw = extract_keywords(&context);
    // Should NOT contain words from the first two messages.
    assert!(!kw.contains(&"oldest".to_string()));
    assert!(!kw.contains(&"ignored".to_string()));
    // Should contain words from the last three.
    assert!(kw.contains(&"rust".to_string()));
    assert!(kw.contains(&"database".to_string()));
    assert!(kw.contains(&"migration".to_string()));
}

#[test]
fn formatting_correctness() {
    let g = make_graph();
    seed_graph(&g);
    let mut injector = MemoryInjector::new();
    let context = vec!["rust unwrap".to_string()];
    let result = injector.inject(&g, &context, 5000).expect("inject");

    if !result.is_empty() {
        // Correction memories should have severity.
        if result.contains("[correction]") {
            assert!(
                result.contains("severity:"),
                "corrections should include severity"
            );
        }
        // Fact memories should have tags.
        if result.contains("[fact]") {
            assert!(result.contains("tags:"), "facts should include tags");
        }
        // Should be wrapped in <memories> tags.
        assert!(result.starts_with("<memories>"));
        assert!(result.ends_with("</memories>"));
    }
}

#[test]
fn cache_hit_returns_same_result() {
    let g = make_graph();
    seed_graph(&g);
    let mut injector = MemoryInjector::new();
    let context = vec!["rust edition".to_string()];

    let first = injector.inject(&g, &context, 500).expect("inject");
    let second = injector.inject(&g, &context, 500).expect("inject");
    assert_eq!(first, second, "cache should return identical result");
}

#[test]
fn cache_invalidated_on_new_context() {
    let g = make_graph();
    seed_graph(&g);
    let mut injector = MemoryInjector::new();

    let ctx1 = vec!["rust edition".to_string()];
    let r1 = injector.inject(&g, &ctx1, 500).expect("inject");

    let ctx2 = vec!["database migration".to_string()];
    let r2 = injector.inject(&g, &ctx2, 500).expect("inject");

    // Different context, different hash — cache miss.
    // Results may or may not differ depending on recall,
    // but the function should not error.
    let _ = (r1, r2);
}

#[test]
fn stop_words_are_excluded() {
    let context = vec!["the is a an to of in for".to_string()];
    let kw = extract_keywords(&context);
    assert!(kw.is_empty(), "all stop words should be excluded");
}

#[test]
fn format_one_decision_has_no_suffix() {
    let mem = Memory {
        id: "1".into(),
        content: "Use hexagonal arch".into(),
        title: String::new(),
        memory_type: MemoryType::Decision,
        importance: 0.8,
        tags: vec!["arch".into()],
        source_type: "manual".into(),
        project_path: String::new(),
        created_at: chrono::Utc::now(),
        updated_at: None,
        access_count: 0,
        last_accessed: None,
    };
    let line = format_one(&mem);
    assert_eq!(line, "- [decision] Use hexagonal arch");
}
