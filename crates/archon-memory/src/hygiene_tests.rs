use super::*;
use crate::extraction::{MAX_EXTRACTED_CONTENT_CHARS, MAX_RULE_CONTENT_CHARS};
use crate::graph::MemoryGraph;

fn store(graph: &MemoryGraph, content: &str, memory_type: MemoryType) -> String {
    graph
        .store_memory(content, "", memory_type, 0.5, &[], "test", "")
        .expect("store")
}

#[test]
fn plan_finds_oversized_rules_and_leaves_ordinary_ones() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let document = "x".repeat(MAX_RULE_CONTENT_CHARS + 1);
    let bloated = store(&graph, &document, MemoryType::Rule);
    let ordinary = store(&graph, "prefer explicit error types", MemoryType::Rule);

    let plan = plan_prune(&graph).expect("plan");

    let ids: Vec<&str> = plan.oversized.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec![bloated.as_str()]);
    assert!(
        !ids.contains(&ordinary.as_str()),
        "a rule within the cap must survive"
    );
    assert!(!plan.applied, "planning must not claim to have applied");
}

/// A `fact` is judged against the looser general cap, not the rule cap.
#[test]
fn plan_uses_the_per_type_cap() {
    let graph = MemoryGraph::in_memory().expect("graph");
    // Longer than a rule may be, but well inside the general limit.
    let mid = "y".repeat(MAX_RULE_CONTENT_CHARS + 50);
    store(&graph, &mid, MemoryType::Fact);
    let huge = "z".repeat(MAX_EXTRACTED_CONTENT_CHARS + 1);
    let oversized = store(&graph, &huge, MemoryType::Fact);

    let plan = plan_prune(&graph).expect("plan");

    let ids: Vec<&str> = plan.oversized.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec![oversized.as_str()]);
}

#[test]
fn plan_groups_duplicates_and_keeps_exactly_one() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let first = store(&graph, "Rust edition must be 2024", MemoryType::Rule);
    // Respaced and recased: the fingerprint normalises both.
    let second = store(&graph, "rust  edition   MUST be 2024", MemoryType::Rule);
    let third = store(&graph, "Rust edition must be 2024", MemoryType::Rule);

    let plan = plan_prune(&graph).expect("plan");

    assert_eq!(plan.duplicates.len(), 1, "all three are one cluster");
    let group = &plan.duplicates[0];
    assert_eq!(group.removed.len(), 2, "exactly one copy is kept");

    let mut seen: Vec<&str> = std::iter::once(group.kept.id.as_str())
        .chain(group.removed.iter().map(|m| m.id.as_str()))
        .collect();
    seen.sort_unstable();
    let mut expected = vec![first.as_str(), second.as_str(), third.as_str()];
    expected.sort_unstable();
    assert_eq!(seen, expected, "the cluster must cover every copy");
}

/// Distinct content must never be merged; over-eager grouping would delete
/// real memories, which is the one outcome worse than the bloat being fixed.
#[test]
fn plan_does_not_group_distinct_content() {
    let graph = MemoryGraph::in_memory().expect("graph");
    store(&graph, "prefer explicit error types", MemoryType::Rule);
    store(&graph, "prefer implicit error types", MemoryType::Rule);

    let plan = plan_prune(&graph).expect("plan");

    assert!(plan.duplicates.is_empty());
    assert!(plan.is_empty());
}

#[test]
fn apply_removes_only_what_the_plan_listed() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let keeper = store(&graph, "a memory worth keeping", MemoryType::Fact);
    store(&graph, "duplicated text", MemoryType::Fact);
    store(&graph, "duplicated text", MemoryType::Fact);
    let bloated = store(
        &graph,
        &"q".repeat(MAX_RULE_CONTENT_CHARS + 1),
        MemoryType::Rule,
    );

    let plan = plan_prune(&graph).expect("plan");
    assert_eq!(plan.removal_count(), 2, "one duplicate and one oversized");

    let deleted = apply_prune(&graph, &plan).expect("apply");
    assert_eq!(deleted, 2);

    assert!(
        graph.get_memory(&keeper).is_ok(),
        "an untouched memory must survive"
    );
    assert!(
        graph.get_memory(&bloated).is_err(),
        "the oversized memory must be gone"
    );
    // Re-planning a pruned store finds nothing: the repair converges.
    assert!(plan_prune(&graph).expect("replan").is_empty());
}

#[test]
fn apply_tolerates_a_memory_deleted_between_plan_and_apply() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let bloated = store(
        &graph,
        &"w".repeat(MAX_RULE_CONTENT_CHARS + 1),
        MemoryType::Rule,
    );

    let plan = plan_prune(&graph).expect("plan");
    graph.delete_memory(&bloated).expect("racing delete");

    let deleted = apply_prune(&graph, &plan).expect("apply must not fail on a missing row");
    assert_eq!(deleted, 0);
}

#[test]
fn report_states_whether_it_is_a_plan_or_a_result() {
    let graph = MemoryGraph::in_memory().expect("graph");
    store(
        &graph,
        &"e".repeat(MAX_RULE_CONTENT_CHARS + 1),
        MemoryType::Rule,
    );

    let mut plan = plan_prune(&graph).expect("plan");
    let planned = format_prune_report(&plan);
    assert!(planned.contains("Would remove"));
    assert!(planned.contains("/memory prune apply"));

    plan.applied = true;
    let applied = format_prune_report(&plan);
    assert!(applied.contains("Removed"));
    assert!(
        !applied.contains("/memory prune apply"),
        "a completed prune must not still be advertising the command"
    );
}

/// Report rows must stay on one line.
///
/// The entries this command exists to surface are pasted documents, so their
/// content is full of newlines. A live run against a real store printed each
/// one across several lines; every fixture here was single-line and missed it.
#[test]
fn excerpt_collapses_newlines_so_report_rows_stay_on_one_line() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let document = format!(
        "# A Heading\n\nA framework for turning a thing\n{}",
        "x".repeat(300)
    );
    store(&graph, &document, MemoryType::Rule);

    let plan = plan_prune(&graph).expect("plan");
    let rendered = format_prune_report(&plan);

    assert_eq!(plan.oversized.len(), 1);
    assert!(
        !plan.oversized[0].excerpt.contains('\n'),
        "excerpt must not carry newlines, got: {:?}",
        plan.oversized[0].excerpt
    );
    assert!(
        plan.oversized[0]
            .excerpt
            .starts_with("# A Heading A framework"),
        "collapsed excerpt should read naturally, got: {:?}",
        plan.oversized[0].excerpt
    );
    // One row per reported memory, plus headings -- no row split across lines.
    let body_lines: Vec<&str> = rendered
        .lines()
        .filter(|line| line.trim_start().starts_with('['))
        .collect();
    assert_eq!(body_lines.len(), 1);
}

#[test]
fn empty_store_reports_nothing_to_do() {
    let graph = MemoryGraph::in_memory().expect("graph");
    let plan = plan_prune(&graph).expect("plan");
    assert!(plan.is_empty());
    assert!(format_prune_report(&plan).contains("Nothing to prune"));
}
