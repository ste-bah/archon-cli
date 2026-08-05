//! Integration tests for the memory garden consolidation module.

use archon_memory::MemoryGraph;
use archon_memory::garden::{GardenConfig, consolidate};
use archon_memory::types::{MemoryType, SearchFilter};

fn make_config() -> GardenConfig {
    GardenConfig {
        auto_consolidate: true,
        min_hours_between_runs: 0, // always run in tests
        dedup_similarity_threshold: 0.85,
        // In-memory test graphs carry no embeddings, so the semantic pass finds
        // no neighbours regardless; the value only has to be present.
        semantic_dedup_max_distance: 0.15,
        semantic_review_max_distance: 0.35,
        staleness_days: 30,
        staleness_importance_floor: 0.3,
        importance_decay_per_day: 0.01,
        max_memories: 5000,
        briefing_limit: 15,
    }
}

// ── 1. garden_consolidate_empty_graph ────────────────────────

#[test]
fn garden_consolidate_empty_graph() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let config = make_config();

    let report = consolidate(&graph, &config).expect("consolidate empty graph");

    assert_eq!(report.duplicates_merged, 0, "no duplicates to merge");
    assert_eq!(report.stale_pruned, 0, "no stale memories to prune");
    assert_eq!(report.importance_decayed, 0, "no importance to decay");
    assert_eq!(report.fragments_merged, 0, "no fragments to merge");
    assert_eq!(report.overflow_pruned, 0, "no overflow to prune");
    // total_memories_after may be 1 due to garden:last_run timestamp
    assert!(
        report.total_memories_after <= 1,
        "expected 0 or 1 memories after consolidation, got {}",
        report.total_memories_after
    );
}

// ── 2. garden_consolidate_preserves_rules ────────────────────

#[test]
fn garden_consolidate_preserves_rules() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let config = make_config();

    // Store 3 Rule memories.
    for i in 1..=3 {
        graph
            .store_memory(
                &format!("Rule number {i}: always do X"),
                &format!("rule-{i}"),
                MemoryType::Rule,
                0.9,
                &["rules".into()],
                "test",
                "/test",
            )
            .expect("store rule memory");
    }

    // Store 2 Fact memories.
    for i in 1..=2 {
        graph
            .store_memory(
                &format!("Fact number {i}: something true"),
                &format!("fact-{i}"),
                MemoryType::Fact,
                0.5,
                &["facts".into()],
                "test",
                "/test",
            )
            .expect("store fact memory");
    }

    consolidate(&graph, &config).expect("consolidate");

    let filter = SearchFilter {
        memory_type: Some(MemoryType::Rule),
        ..SearchFilter::default()
    };
    let rules = graph.search_memories(&filter).expect("search rules");
    assert_eq!(rules.len(), 3, "all 3 rules should survive consolidation");
}

// ── 3. garden_consolidate_preserves_personality_snapshots ────

#[test]
fn garden_consolidate_preserves_personality_snapshots() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let config = make_config();

    for i in 1..=2 {
        graph
            .store_memory(
                &format!("Personality snapshot {i}: friendly and helpful"),
                &format!("snap-{i}"),
                MemoryType::PersonalitySnapshot,
                0.8,
                &["personality".into()],
                "test",
                "/test",
            )
            .expect("store personality snapshot");
    }

    consolidate(&graph, &config).expect("consolidate");

    let filter = SearchFilter {
        memory_type: Some(MemoryType::PersonalitySnapshot),
        ..SearchFilter::default()
    };
    let snaps = graph
        .search_memories(&filter)
        .expect("search personality snapshots");
    assert_eq!(
        snaps.len(),
        2,
        "both personality snapshots should survive consolidation"
    );
}

// ── 4. garden_dedup_merges_near_duplicates ───────────────────

#[test]
fn garden_dedup_merges_near_duplicates() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let config = make_config();

    graph
        .store_memory(
            "Rust uses borrow checker for memory safety in systems programming",
            "borrow-1",
            MemoryType::Fact,
            0.7,
            &["rust".into(), "safety".into()],
            "test",
            "/test",
        )
        .expect("store first fact");

    graph
        .store_memory(
            "Rust uses borrow checker for memory safety in systems programming languages",
            "borrow-2",
            MemoryType::Fact,
            0.6,
            &["rust".into(), "borrow".into()],
            "test",
            "/test",
        )
        .expect("store second fact");

    let report = consolidate(&graph, &config).expect("consolidate");
    assert!(
        report.duplicates_merged >= 1,
        "expected at least 1 duplicate merged, got {}",
        report.duplicates_merged
    );

    let filter = SearchFilter {
        memory_type: Some(MemoryType::Fact),
        ..SearchFilter::default()
    };
    let facts = graph.search_memories(&filter).expect("search facts");
    // Filter out the garden:last_run entry which is also a Fact.
    let non_garden_facts: Vec<_> = facts
        .iter()
        .filter(|m| !m.tags.contains(&"garden:last_run".to_string()))
        .collect();

    assert_eq!(
        non_garden_facts.len(),
        1,
        "only 1 fact should remain after dedup, got {}",
        non_garden_facts.len()
    );

    // Verify merged tags contain tags from both originals.
    let survivor = &non_garden_facts[0];
    assert!(
        survivor.tags.contains(&"rust".to_string()),
        "survivor should have 'rust' tag"
    );
    assert!(
        survivor.tags.contains(&"safety".to_string())
            || survivor.tags.contains(&"borrow".to_string()),
        "survivor should have merged tags from victim"
    );

    // The dedup phase leaves a `Supersedes` edge behind, and the fragment
    // phase that runs after it reads relationships. Without a guard it folds
    // the losing half straight back into the survivor, whose content then
    // reads as its own text twice -- in the prompt, on every recall. Undoing
    // the merge in the same run that made it.
    assert!(
        !survivor.content.contains(" | "),
        "the survivor must not have the superseded duplicate concatenated \
         back into it, got: {}",
        survivor.content
    );
    assert_eq!(
        survivor.content, "Rust uses borrow checker for memory safety in systems programming",
        "the survivor keeps its own content unchanged"
    );
}

// ── 5. garden_dedup_preserves_distinct ───────────────────────

#[test]
fn garden_dedup_preserves_distinct() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let config = make_config();

    graph
        .store_memory(
            "Rust is a systems language",
            "fact-rust",
            MemoryType::Fact,
            0.7,
            &["rust".into()],
            "test",
            "/test",
        )
        .expect("store rust fact");

    graph
        .store_memory(
            "Python is good for data science",
            "fact-python",
            MemoryType::Fact,
            0.7,
            &["python".into()],
            "test",
            "/test",
        )
        .expect("store python fact");

    consolidate(&graph, &config).expect("consolidate");

    let filter = SearchFilter {
        memory_type: Some(MemoryType::Fact),
        ..SearchFilter::default()
    };
    let facts = graph.search_memories(&filter).expect("search facts");
    let non_garden_facts: Vec<_> = facts
        .iter()
        .filter(|m| !m.tags.contains(&"garden:last_run".to_string()))
        .collect();

    assert_eq!(
        non_garden_facts.len(),
        2,
        "both distinct facts should survive, got {}",
        non_garden_facts.len()
    );
}

// ── 6. garden_overflow_prune_respects_max ────────────────────

#[test]
fn garden_overflow_prune_respects_max() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let mut config = make_config();
    config.max_memories = 5;

    // Store 8 facts with varying importance.
    for i in 1..=8 {
        graph
            .store_memory(
                &format!("Overflow fact number {i} with unique content about topic {i}"),
                &format!("overflow-{i}"),
                MemoryType::Fact,
                i as f64 * 0.1, // 0.1, 0.2, ..., 0.8
                &[format!("tag-{i}")],
                "test",
                "/test",
            )
            .expect("store overflow fact");
    }

    let report = consolidate(&graph, &config).expect("consolidate");

    assert!(
        report.overflow_pruned > 0,
        "expected some overflow pruning, got 0"
    );
    // max_memories=5 + 1 garden:last_run sentinel = 6 possible
    assert!(
        report.total_memories_after <= 6,
        "expected at most 6 memories after overflow prune (5 + garden:last_run), got {}",
        report.total_memories_after
    );

    // Verify the lowest-importance memories were removed.
    // The highest-importance facts (0.8, 0.7, 0.6, 0.5) should survive.
    let filter = SearchFilter {
        memory_type: Some(MemoryType::Fact),
        ..SearchFilter::default()
    };
    let remaining = graph.search_memories(&filter).expect("search remaining");
    let non_garden: Vec<_> = remaining
        .iter()
        .filter(|m| !m.tags.contains(&"garden:last_run".to_string()))
        .collect();

    for m in &non_garden {
        assert!(
            m.importance >= 0.4,
            "low-importance memory (importance={}) should have been pruned",
            m.importance
        );
    }
}

// ── 7. garden_overflow_prune_skips_rules ─────────────────────

#[test]
fn garden_overflow_prune_skips_rules() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let mut config = make_config();
    config.max_memories = 3;

    // Store 2 Rule memories with high importance.
    for i in 1..=2 {
        graph
            .store_memory(
                &format!("Important rule {i}: never do Y"),
                &format!("rule-{i}"),
                MemoryType::Rule,
                0.95,
                &["rules".into()],
                "test",
                "/test",
            )
            .expect("store rule");
    }

    // Store 4 Fact memories with low importance.
    for i in 1..=4 {
        graph
            .store_memory(
                &format!("Low importance fact {i} about unrelated topic {i}"),
                &format!("low-fact-{i}"),
                MemoryType::Fact,
                0.1,
                &[format!("low-{i}")],
                "test",
                "/test",
            )
            .expect("store low-importance fact");
    }

    consolidate(&graph, &config).expect("consolidate");

    // Both rules must survive.
    let filter = SearchFilter {
        memory_type: Some(MemoryType::Rule),
        ..SearchFilter::default()
    };
    let rules = graph.search_memories(&filter).expect("search rules");
    assert_eq!(rules.len(), 2, "both rules must survive overflow pruning");
}
