//! Apply and rollback, checked against a real store by reading it back.
//!
//! Every rollback test asserts the memory is visible to ORDINARY reads again,
//! not merely that the tag is gone. A tag assertion would pass on a build where
//! the read path had stopped honouring the tag in the first place.

use super::{
    ChangeOutcome, apply_memory_retirement, apply_rule_retirement, apply_semantic_consolidation,
    derived_memory_id, rollback_memory_retirement, rollback_rule_retirement,
    rollback_semantic_consolidation,
};
use crate::MemoryGraph;
use crate::access::MemoryTrait;
use crate::garden::consolidation::{ConsolidationSource, SemanticConsolidationCandidate};
use crate::types::{MemoryType, SearchFilter};

fn store() -> MemoryGraph {
    MemoryGraph::in_memory().expect("graph")
}

fn visible_ids(graph: &MemoryGraph, memory_type: MemoryType) -> Vec<String> {
    graph
        .search_memories(&SearchFilter {
            memory_type: Some(memory_type),
            ..SearchFilter::default()
        })
        .expect("search")
        .into_iter()
        .map(|memory| memory.id)
        .collect()
}

#[test]
fn retiring_a_memory_hides_it_from_reads_without_destroying_it() {
    let graph = store();
    let id = graph
        .store_memory("a fact", "t", MemoryType::Fact, 0.5, &[], "test", "")
        .expect("store");

    assert_eq!(
        apply_memory_retirement(&graph, &id).expect("apply"),
        ChangeOutcome::Changed
    );

    assert!(
        !visible_ids(&graph, MemoryType::Fact).contains(&id),
        "a retired memory must leave ordinary search"
    );
    assert!(
        graph.inspect_memory(&id).is_ok(),
        "the row must still exist, or rollback has nothing to restore"
    );
}

#[test]
fn rolling_back_a_retirement_brings_the_memory_back_to_ordinary_reads() {
    // The claim that makes retirement acceptable at all: it is undoable.
    let graph = store();
    let id = graph
        .store_memory("a fact", "t", MemoryType::Fact, 0.5, &[], "test", "")
        .expect("store");
    let before = graph.inspect_memory(&id).expect("read before");

    apply_memory_retirement(&graph, &id).expect("apply");
    assert_eq!(
        rollback_memory_retirement(&graph, &id).expect("rollback"),
        ChangeOutcome::Changed
    );

    assert!(
        visible_ids(&graph, MemoryType::Fact).contains(&id),
        "a rolled-back memory must be findable by ordinary search again"
    );
    let after = graph.inspect_memory(&id).expect("read after");
    assert_eq!(after.content, before.content);
    assert_eq!(after.importance, before.importance);
    assert_eq!(after.tags, before.tags, "the row must be exactly as it was");
}

#[test]
fn apply_and_rollback_are_both_idempotent() {
    // A retry must be able to say "already done" without claiming to have done
    // it: a governed record counting replays as fresh applications would
    // inflate the numbers a promotion gate reads.
    let graph = store();
    let id = graph
        .store_memory("a fact", "t", MemoryType::Fact, 0.5, &[], "test", "")
        .expect("store");

    assert_eq!(
        apply_memory_retirement(&graph, &id).expect("first"),
        ChangeOutcome::Changed
    );
    assert_eq!(
        apply_memory_retirement(&graph, &id).expect("second"),
        ChangeOutcome::AlreadyInPlace
    );
    assert_eq!(
        rollback_memory_retirement(&graph, &id).expect("first undo"),
        ChangeOutcome::Changed
    );
    assert_eq!(
        rollback_memory_retirement(&graph, &id).expect("second undo"),
        ChangeOutcome::AlreadyInPlace
    );
}

#[test]
fn retiring_a_rule_removes_it_from_rule_listings_and_rollback_restores_its_score() {
    // Rules are read back through the same search path as every other memory,
    // so the tag is enough to take a rule out of the prompt block without
    // touching its text or score. Rollback must return it with the score it had
    // -- a re-created rule would start from the default and lose everything the
    // corrections behind it had earned.
    let graph = store();
    let rule_id = graph
        .store_memory(
            "check constraints before acting",
            "",
            MemoryType::Rule,
            73.0,
            &["source:correction_derived".to_string()],
            "rules_engine",
            "",
        )
        .expect("store rule");

    apply_rule_retirement(&graph, &rule_id).expect("apply");
    assert!(
        !visible_ids(&graph, MemoryType::Rule).contains(&rule_id),
        "a retired rule must not be listed for the prompt"
    );

    rollback_rule_retirement(&graph, &rule_id).expect("rollback");
    let restored = graph.inspect_memory(&rule_id).expect("read");
    assert!(visible_ids(&graph, MemoryType::Rule).contains(&rule_id));
    assert_eq!(
        restored.importance, 73.0,
        "the rule must come back with the score its corrections earned"
    );
    assert_eq!(restored.content, "check constraints before acting");
}

#[test]
fn the_two_retirement_paths_refuse_each_others_rows() {
    // A mis-typed proposal must not take a rule out of the prompt through the
    // memory path, nor hide an ordinary memory through the rule path.
    let graph = store();
    let fact = graph
        .store_memory("a fact", "t", MemoryType::Fact, 0.5, &[], "test", "")
        .expect("store fact");
    let rule = graph
        .store_memory(
            "a rule",
            "",
            MemoryType::Rule,
            50.0,
            &[],
            "rules_engine",
            "",
        )
        .expect("store rule");

    assert!(apply_memory_retirement(&graph, &rule).is_err());
    assert!(apply_rule_retirement(&graph, &fact).is_err());
    assert!(
        visible_ids(&graph, MemoryType::Rule).contains(&rule),
        "a refused apply must leave the store untouched"
    );
    assert!(visible_ids(&graph, MemoryType::Fact).contains(&fact));
}

fn candidate(sources: &[(&str, f64)]) -> SemanticConsolidationCandidate {
    SemanticConsolidationCandidate {
        candidate_id: "cand-1".into(),
        proposed_content: "always run the formatter before committing".into(),
        proposed_title: "formatting".into(),
        memory_type: MemoryType::Fact,
        project_path: String::new(),
        source_type: "extraction".into(),
        proposed_importance: 0.8,
        representative_id: sources[0].0.to_string(),
        sources: sources
            .iter()
            .map(|(id, importance)| ConsolidationSource {
                memory_id: (*id).to_string(),
                excerpt: "always run the formatter".into(),
                importance: *importance,
                created_at: chrono::Utc::now(),
            })
            .collect(),
    }
}

#[test]
fn applying_a_consolidation_writes_the_memory_and_its_provenance_edges() {
    let graph = store();
    let mut ids = Vec::new();
    for index in 0..3 {
        ids.push(
            graph
                .store_memory(
                    &format!("always run the formatter, wording {index}"),
                    "formatting",
                    MemoryType::Fact,
                    0.5,
                    &[],
                    "extraction",
                    "",
                )
                .expect("store"),
        );
    }
    let refs: Vec<(&str, f64)> = ids.iter().map(|id| (id.as_str(), 0.5)).collect();

    let (derived, outcome) =
        apply_semantic_consolidation(&graph, &candidate(&refs), "run-1").expect("apply");

    assert_eq!(outcome, ChangeOutcome::Changed);
    assert_eq!(derived, derived_memory_id("cand-1"));
    let stored = graph.inspect_memory(&derived).expect("read derived");
    assert_eq!(
        stored.content, "always run the formatter before committing",
        "the applied memory must carry the proposed text verbatim"
    );
    let related = graph.get_related_memories(&derived, 1).expect("related");
    assert_eq!(
        related.len(),
        3,
        "every source must be reachable from the derived memory"
    );
    for id in &ids {
        assert!(
            graph.inspect_memory(id).is_ok(),
            "consolidation adds a claim; it must not retire the evidence"
        );
    }
}

#[test]
fn re_applying_a_consolidation_does_not_mint_a_second_memory() {
    let graph = store();
    let source = graph
        .store_memory(
            "always run the formatter",
            "f",
            MemoryType::Fact,
            0.5,
            &[],
            "extraction",
            "",
        )
        .expect("store");
    let candidate = candidate(&[(source.as_str(), 0.5)]);

    let (first, first_outcome) =
        apply_semantic_consolidation(&graph, &candidate, "run-1").expect("first");
    let (second, second_outcome) =
        apply_semantic_consolidation(&graph, &candidate, "run-2").expect("second");

    assert_eq!(first, second);
    assert_eq!(first_outcome, ChangeOutcome::Changed);
    assert_eq!(second_outcome, ChangeOutcome::AlreadyInPlace);
}

#[test]
fn rolling_back_a_consolidation_withdraws_the_derived_memory_and_keeps_the_sources() {
    let graph = store();
    let source = graph
        .store_memory(
            "always run the formatter",
            "f",
            MemoryType::Fact,
            0.5,
            &[],
            "extraction",
            "",
        )
        .expect("store");
    let candidate = candidate(&[(source.as_str(), 0.5)]);
    let (derived, _) = apply_semantic_consolidation(&graph, &candidate, "run-1").expect("apply");

    rollback_semantic_consolidation(&graph, &derived).expect("rollback");

    assert!(
        !visible_ids(&graph, MemoryType::Fact).contains(&derived),
        "the derived memory must leave ordinary reads"
    );
    assert!(
        graph.inspect_memory(&derived).is_ok(),
        "the row stays so its provenance edges keep pointing at something real"
    );
    assert!(
        visible_ids(&graph, MemoryType::Fact).contains(&source),
        "rolling back a consolidation must not disturb its sources"
    );
}

#[test]
fn rollback_refuses_a_memory_consolidation_did_not_write() {
    // Without this, a rollback pointed at the wrong id becomes a way to retire
    // any memory in the store with no proposal and no decision behind it.
    let graph = store();
    let ordinary = graph
        .store_memory("a fact", "t", MemoryType::Fact, 0.5, &[], "test", "")
        .expect("store");

    assert!(rollback_semantic_consolidation(&graph, &ordinary).is_err());
    assert!(visible_ids(&graph, MemoryType::Fact).contains(&ordinary));
}

#[test]
fn a_retired_memory_is_invisible_to_recall_as_well_as_search() {
    // Search, recall and listing are three read paths. A tag honoured by only
    // one of them is a memory that still reaches the prompt.
    let graph = store();
    let id = graph
        .store_memory(
            "the formatter must run before committing",
            "formatting",
            MemoryType::Fact,
            0.9,
            &[],
            "test",
            "",
        )
        .expect("store");
    apply_memory_retirement(&graph, &id).expect("apply");

    let recalled = graph
        .recall_memories("formatter committing", 10)
        .expect("recall");
    assert!(
        !recalled.iter().any(|memory| memory.id == id),
        "a retired memory came back from recall"
    );
    let listed = graph.list_recent(50).expect("list");
    assert!(
        !listed.iter().any(|memory| memory.id == id),
        "a retired memory came back from listing"
    );
}
