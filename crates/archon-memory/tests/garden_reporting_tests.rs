//! Scheduling, briefing, and report formatting for the memory garden.
//!
//! Split from `garden_tests.rs` at the 500-line gate. The seam is subject:
//! everything here observes or reports on consolidation, while the tests left
//! behind exercise the passes that MUTATE the graph.

//! Integration tests for the memory garden consolidation module.

use archon_memory::MemoryGraph;
use archon_memory::garden::{
    GardenConfig, GardenReport, consolidate, format_garden_stats, generate_briefing,
    should_auto_consolidate,
};
use archon_memory::types::MemoryType;

fn make_config() -> GardenConfig {
    GardenConfig {
        auto_consolidate: true,
        min_hours_between_runs: 0, // always run in tests
        dedup_similarity_threshold: 0.85,
        // In-memory test graphs carry no embeddings, so the semantic pass finds
        // no neighbours regardless; the value only has to be present.
        semantic_dedup_max_distance: 0.15,
        semantic_review_max_distance: 0.35,
        // Consolidation itself never adjudicates; the caller does, after the
        // report comes back. Held at the shipped default so these tests keep
        // exercising the shipped shape.
        auto_adjudicate_review_band: false,
        auto_adjudicate_min_pairs: 10,
        staleness_days: 30,
        staleness_importance_floor: 0.3,
        importance_decay_per_day: 0.01,
        max_memories: 5000,
        briefing_limit: 15,
    }
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
        review_pairs: Vec::new(),
        semantic_pass_unavailable: false,
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

// ── 17. semantic pass availability is reported, not assumed ─

/// A store with no vector search reports the semantic pass as UNAVAILABLE.
///
/// The report used to say `duplicates_merged: 0` and stop there, which reads as
/// "examined, nothing to merge". Every Archon process after the first is in
/// exactly this position -- CozoDB admits one writer, so the rest read memory
/// over TCP -- so the common case was a pass that never ran being reported as a
/// clean store.
#[test]
fn garden_reports_semantic_pass_unavailable_without_a_vector_index() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    for i in 1..=2 {
        graph
            .store_memory(
                &format!("Distinct subject {i}"),
                &format!("s-{i}"),
                MemoryType::Fact,
                0.6,
                &[],
                "test",
                "/test",
            )
            .expect("store fact");
    }

    let report = consolidate(&graph, &make_config()).expect("consolidate");

    assert!(
        report.semantic_pass_unavailable,
        "an unindexed store must report the semantic pass as unavailable"
    );
    assert!(
        report.format().contains("unavailable"),
        "the human-readable report must say so too, got:\n{}",
        report.format()
    );
}

/// A store WITH vector search still reports ordinary counts.
///
/// The guard against fixing the case above by inverting it: an available pass
/// that merged something must not be reported as unavailable.
#[test]
fn garden_reports_counts_normally_with_a_vector_index() {
    let graph = MemoryGraph::in_memory().expect("create in-memory graph");
    archon_memory::vector_search::init_embedding_schema(graph.db(), 4).expect("embedding schema");

    let store = |content: &str, importance: f64| {
        graph
            .store_memory(content, "", MemoryType::Fact, importance, &[], "test", "")
            .expect("store")
    };
    let anchor = store("deploy to eu-west-2", 0.9);
    let paraphrase = store("target the eu-west-2 region", 0.4);

    // Hand-built vectors so the geometry is exact rather than model-dependent.
    let put = |id: &str, v: [f32; 4]| {
        archon_memory::vector_search::store_embedding(graph.db(), id, &v, "test", 4)
            .expect("embedding")
    };
    put(&anchor, [1.0, 0.0, 0.0, 0.0]);
    put(&paraphrase, [0.99, 0.09, 0.0, 0.0]);

    let report = consolidate(&graph, &make_config()).expect("consolidate");

    assert!(
        !report.semantic_pass_unavailable,
        "a store with a live vector index must not report the pass as unavailable"
    );
    assert_eq!(
        report.duplicates_merged, 1,
        "the paraphrase must be merged and counted"
    );
    assert!(
        !report.format().contains("unavailable"),
        "an available pass must not print the unavailable notice, got:\n{}",
        report.format()
    );
}
