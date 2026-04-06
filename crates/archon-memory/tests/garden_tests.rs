//! Integration tests for the memory garden consolidation module.

use archon_memory::MemoryGraph;
use archon_memory::garden::{
    GardenConfig, GardenReport, consolidate, format_garden_stats, generate_briefing,
    should_auto_consolidate,
};
use archon_memory::types::{MemoryType, SearchFilter};

fn make_config() -> GardenConfig {
    GardenConfig {
        auto_consolidate: true,
        min_hours_between_runs: 0, // always run in tests
        dedup_similarity_threshold: 0.85,
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

// ── 8. garden_should_auto_consolidate_first_run ──────────────

#[test]
fn garden_should_auto_consolidate_first_run() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");

    let result =
        should_auto_consolidate(&graph, 24).expect("should_auto_consolidate on empty graph");
    assert!(
        result,
        "should_auto_consolidate should return true on first run (no timestamp)"
    );
}

// ── 9. garden_should_auto_consolidate_after_run ──────────────

#[test]
fn garden_should_auto_consolidate_after_run() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let config = make_config();

    // Run consolidation to record timestamp.
    consolidate(&graph, &config).expect("consolidate");

    // With 24-hour minimum, should return false (just ran).
    let result =
        should_auto_consolidate(&graph, 24).expect("should_auto_consolidate after recent run");
    assert!(
        !result,
        "should_auto_consolidate should return false immediately after consolidation with 24h min"
    );
}

// ── 10. garden_should_auto_consolidate_with_zero_hours ───────

#[test]
fn garden_should_auto_consolidate_with_zero_hours() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let config = make_config();

    // Run consolidation to record timestamp.
    consolidate(&graph, &config).expect("consolidate");

    // With 0-hour minimum, should always return true.
    let result = should_auto_consolidate(&graph, 0).expect("should_auto_consolidate with 0 hours");
    assert!(
        result,
        "should_auto_consolidate should return true with min_hours=0"
    );
}

// ── 11. garden_generate_briefing_format ──────────────────────

#[test]
fn garden_generate_briefing_format() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");

    // Store 3 facts.
    for i in 1..=3 {
        graph
            .store_memory(
                &format!("Briefing fact {i}: important information"),
                &format!("brief-fact-{i}"),
                MemoryType::Fact,
                0.7,
                &["briefing".into()],
                "test",
                "/test",
            )
            .expect("store briefing fact");
    }

    // Store 2 rules.
    for i in 1..=2 {
        graph
            .store_memory(
                &format!("Briefing rule {i}: always follow this"),
                &format!("brief-rule-{i}"),
                MemoryType::Rule,
                0.9,
                &["briefing".into()],
                "test",
                "/test",
            )
            .expect("store briefing rule");
    }

    let briefing = generate_briefing(&graph, 15).expect("generate briefing");

    assert!(
        briefing.contains("<memory_briefing>"),
        "briefing should contain opening tag"
    );
    assert!(
        briefing.contains("</memory_briefing>"),
        "briefing should contain closing tag"
    );
    assert!(
        briefing.contains("Memory graph:"),
        "briefing should contain 'Memory graph:'"
    );
    assert!(
        briefing.contains("Key memories:"),
        "briefing should contain 'Key memories:'"
    );
}

// ── 12. garden_generate_briefing_empty_graph ─────────────────

#[test]
fn garden_generate_briefing_empty_graph() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");

    let briefing = generate_briefing(&graph, 15).expect("generate briefing on empty graph");

    assert!(
        briefing.contains("<memory_briefing>"),
        "briefing should contain opening tag even on empty graph"
    );
    assert!(
        briefing.contains("</memory_briefing>"),
        "briefing should contain closing tag even on empty graph"
    );
    assert!(
        briefing.contains("Memory graph:"),
        "briefing should contain 'Memory graph:' even on empty graph"
    );
    assert!(
        briefing.contains("0 memories"),
        "briefing should mention '0 memories' for empty graph"
    );
}

// ── 13. garden_fresh_memories_not_decayed ────────────────────

#[test]
fn garden_fresh_memories_not_decayed() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let config = make_config();

    let id = graph
        .store_memory(
            "Fresh fact that should not decay",
            "fresh-fact",
            MemoryType::Fact,
            0.5,
            &["fresh".into()],
            "test",
            "/test",
        )
        .expect("store fresh fact");

    consolidate(&graph, &config).expect("consolidate");

    let mem = graph
        .get_memory(&id)
        .expect("get fresh fact after consolidation");
    assert!(
        (mem.importance - 0.5).abs() < f64::EPSILON,
        "fresh memory importance should remain 0.5, got {}",
        mem.importance
    );
}

// ── 14. garden_idempotent ────────────────────────────────────

#[test]
fn garden_idempotent() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    let config = make_config();

    // Seed with some data.
    for i in 1..=3 {
        graph
            .store_memory(
                &format!("Idempotent fact {i} with distinct content about subject {i}"),
                &format!("idem-{i}"),
                MemoryType::Fact,
                0.6,
                &[format!("idem-{i}")],
                "test",
                "/test",
            )
            .expect("store idempotent fact");
    }

    // First consolidation.
    let report1 = consolidate(&graph, &config).expect("first consolidation");

    // Second consolidation.
    let report2 = consolidate(&graph, &config).expect("second consolidation");

    assert_eq!(
        report1.total_memories_after, report2.total_memories_after,
        "total_memories_after should be the same between runs"
    );
    assert_eq!(
        report2.duplicates_merged, 0,
        "second run should merge 0 duplicates"
    );
    assert_eq!(
        report2.stale_pruned, 0,
        "second run should prune 0 stale memories"
    );
    assert_eq!(
        report2.overflow_pruned, 0,
        "second run should prune 0 overflow memories"
    );
    assert_eq!(
        report2.fragments_merged, 0,
        "second run should merge 0 fragments"
    );
}

// ── 15. garden_report_format (TASK-CLI-417) ─────────────────

#[test]
fn garden_report_format() {
    let report = GardenReport {
        duplicates_merged: 3,
        stale_pruned: 12,
        importance_decayed: 47,
        fragments_merged: 2,
        overflow_pruned: 0,
        total_memories_before: 892,
        total_memories_after: 875,
        duration_ms: 342,
    };
    let formatted = report.format();
    assert!(
        formatted.contains("Consolidation Complete"),
        "should contain header"
    );
    assert!(formatted.contains("3"), "should show duplicates count");
    assert!(formatted.contains("12"), "should show stale pruned count");
    assert!(formatted.contains("47"), "should show decayed count");
    assert!(formatted.contains("892"), "should show before count");
    assert!(formatted.contains("875"), "should show after count");
    assert!(formatted.contains("342ms"), "should show duration");
}

// ── 16. garden_stats_format (TASK-CLI-417) ──────────────────

#[test]
fn garden_stats_format() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");

    for i in 1..=3 {
        graph
            .store_memory(
                &format!("Fact {i}"),
                &format!("fact-{i}"),
                MemoryType::Fact,
                0.7,
                &[],
                "test",
                "/test",
            )
            .expect("store fact");
    }
    graph
        .store_memory(
            "Decision 1",
            "dec-1",
            MemoryType::Decision,
            0.8,
            &[],
            "test",
            "/test",
        )
        .expect("store decision");

    let stats = format_garden_stats(&graph, 10).expect("format stats");
    assert!(stats.contains("Statistics"), "should contain header");
    assert!(stats.contains("Total memories:"), "should show total");
    assert!(stats.contains("Fact"), "should show Fact type");
    assert!(stats.contains("By type:"), "should have type section");
}
