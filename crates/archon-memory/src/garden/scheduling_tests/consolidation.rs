//! What a scheduled pass proposes for consolidation, and what it declines to.
//!
//! Split from the parent test module to stay under the file-size gate; the
//! store helpers it shares live there and are reached through `super`.

use super::*;

#[test]
fn a_memory_proposed_for_retirement_is_not_also_proposed_for_consolidation() {
    // Both proposals would be true on their own evidence: the rows are stale,
    // and they do restate one another. Approving both would retire the sources
    // while promoting their content to a fresh durable memory -- a strange way
    // to honour a decision to let something go.
    let graph = MemoryGraph::in_memory().expect("create graph");
    for index in 0..4 {
        let id = graph
            .store_memory(
                &format!("always run the formatter before committing, wording {index}"),
                "formatting",
                MemoryType::Fact,
                0.1,
                &[],
                "extraction",
                "",
            )
            .expect("store");
        age_memory(&graph, &id, 90);
    }
    let dir = tempfile::tempdir().expect("tempdir");

    let report = run_scheduled_consolidation(&graph, &scheduled_config(), dir.path(), "run-both")
        .expect("scheduled pass runs")
        .report()
        .expect("ran");

    assert_eq!(report.retirement_candidates.len(), 4);
    let retiring: std::collections::HashSet<&str> = report
        .retirement_candidates
        .iter()
        .map(|candidate| candidate.memory_id.as_str())
        .collect();
    for candidate in &report.consolidation_candidates {
        for source in &candidate.sources {
            assert!(
                !retiring.contains(source.memory_id.as_str()),
                "{} is proposed for retirement and as consolidation evidence",
                source.memory_id
            );
        }
    }
}

#[test]
fn a_live_cluster_is_proposed_for_consolidation() {
    // The other half: consolidation must actually fire on memories that are not
    // going anywhere, or the exclusion above would be indistinguishable from a
    // phase that never proposes.
    let graph = MemoryGraph::in_memory().expect("create graph");
    for index in 0..3 {
        graph
            .store_memory(
                &format!("always run the formatter before committing, wording {index}"),
                "formatting",
                MemoryType::Fact,
                0.9,
                &[],
                "extraction",
                "",
            )
            .expect("store");
    }
    let dir = tempfile::tempdir().expect("tempdir");

    let report = run_scheduled_consolidation(&graph, &scheduled_config(), dir.path(), "run-live")
        .expect("scheduled pass runs")
        .report()
        .expect("ran");

    assert!(report.retirement_candidates.is_empty());
    assert_eq!(report.consolidation_candidates.len(), 1);
    assert_eq!(report.consolidation_candidates[0].corroboration_count(), 3);
    assert_eq!(
        graph.memory_count().expect("count"),
        4,
        "proposing a consolidation must not write the memory it proposes"
    );
}

#[test]
fn the_interactive_pass_proposes_no_consolidations() {
    // `/garden` reports what it DID. A list of proposals nobody can act on from
    // that surface would read as work performed.
    let graph = MemoryGraph::in_memory().expect("create graph");
    for index in 0..3 {
        graph
            .store_memory(
                &format!("always run the formatter before committing, wording {index}"),
                "formatting",
                MemoryType::Fact,
                0.9,
                &[],
                "extraction",
                "",
            )
            .expect("store");
    }

    let report = consolidate_with_policy(
        &graph,
        &GardenConfig::default(),
        "manual",
        GardenRunPolicy::interactive(),
    )
    .expect("manual pass runs");

    assert!(report.consolidation_candidates.is_empty());
}

#[test]
fn the_retirement_candidate_ceiling_bounds_the_review_pile() {
    let graph = MemoryGraph::in_memory().expect("create graph");
    for i in 0..8 {
        let id = graph
            .store_memory(
                &format!("forgotten memory number {i}"),
                "old",
                MemoryType::Fact,
                0.1,
                &[],
                "test",
                "",
            )
            .expect("store");
        age_memory(&graph, &id, 90);
    }
    let config = GardenConfig {
        scheduled_max_retirement_candidates: 3,
        ..scheduled_config()
    };
    let dir = tempfile::tempdir().expect("tempdir");

    let report = run_scheduled_consolidation(&graph, &config, dir.path(), "run-cap")
        .expect("scheduled pass runs")
        .report()
        .expect("ran");

    assert_eq!(report.retirement_candidates.len(), 3);
    assert!(report.budget_exhausted);
    assert_eq!(
        graph.memory_count().expect("count"),
        9,
        "capping the review pile must not cost a single memory"
    );
}
