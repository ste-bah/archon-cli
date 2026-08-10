//! Reading rule evidence out of correction rows.
//!
//! The assertions that matter are about the direction of error: a rule with no
//! evidence must read as quiet, and a rule whose evidence cannot be read must
//! NOT, because silence is what retires a rule.

use std::sync::Arc;

use archon_consciousness::rules::{RuleSource, RulesEngine};
use archon_memory::garden::RuleOrigin;
use archon_memory::types::{MemoryError, MemoryType, SearchFilter};
use archon_memory::{MemoryGraph, MemoryTrait};

use super::rule_observations;

fn store() -> MemoryGraph {
    MemoryGraph::in_memory().expect("graph")
}

/// Record a correction naming `rule_id`, the way the tracker does.
fn correction_for(graph: &MemoryGraph, rule_id: &str, content: &str) {
    graph
        .store_memory(
            content,
            "correction",
            MemoryType::Correction,
            10.0,
            &[format!("target-rule:{rule_id}")],
            "correction_tracker",
            "turn:1",
        )
        .expect("store correction");
}

#[test]
fn a_rule_with_no_corrections_reads_as_having_none() {
    let graph = store();
    let engine = RulesEngine::new(&graph);
    let rule = engine
        .add_rule(
            "check constraints before acting",
            RuleSource::CorrectionDerived,
        )
        .expect("add rule");

    let observations = rule_observations(&graph);

    let observed = observations
        .iter()
        .find(|observation| observation.rule_id == rule.id)
        .expect("the rule should be observed");
    assert_eq!(observed.supporting_corrections, 0);
    assert_eq!(observed.most_recent_correction, None);
    assert_eq!(observed.origin, RuleOrigin::CorrectionDerived);
}

#[test]
fn corrections_naming_a_rule_are_counted_and_dated() {
    // Provenance comes from the correction rows, not from rule text -- every
    // correction-derived rule of a given category has identical text.
    let graph = store();
    let engine = RulesEngine::new(&graph);
    let rule = engine
        .add_rule(
            "check constraints before acting",
            RuleSource::CorrectionDerived,
        )
        .expect("add rule");
    correction_for(&graph, &rule.id, "you did that without asking");
    correction_for(&graph, &rule.id, "again, without asking");

    let observations = rule_observations(&graph);

    let observed = observations
        .iter()
        .find(|observation| observation.rule_id == rule.id)
        .expect("observed");
    assert_eq!(observed.supporting_corrections, 2);
    assert!(observed.most_recent_correction.is_some());
}

#[test]
fn a_correction_naming_another_rule_is_not_counted() {
    let graph = store();
    let engine = RulesEngine::new(&graph);
    let mine = engine
        .add_rule("check constraints", RuleSource::CorrectionDerived)
        .expect("add");
    let other = engine
        .add_rule(
            "verify claims against evidence",
            RuleSource::CorrectionDerived,
        )
        .expect("add");
    correction_for(&graph, &other.id, "that fact was wrong");

    let observations = rule_observations(&graph);

    let observed = observations
        .iter()
        .find(|observation| observation.rule_id == mine.id)
        .expect("observed");
    assert_eq!(
        observed.supporting_corrections, 0,
        "evidence for one rule must not be attributed to another"
    );
}

#[test]
fn rule_origin_is_carried_across_faithfully() {
    // Only correction-derived rules are ever proposed for retirement, so a
    // mis-mapped origin would put a user's own rule at risk.
    let graph = store();
    let engine = RulesEngine::new(&graph);
    engine
        .add_rule("a rule the user typed", RuleSource::UserDefined)
        .expect("add");
    engine
        .add_rule("a rule that shipped", RuleSource::SystemDefault)
        .expect("add");
    engine
        .add_rule("a rule a correction made", RuleSource::CorrectionDerived)
        .expect("add");

    let observations = rule_observations(&graph);

    let origins: Vec<RuleOrigin> = observations.iter().map(|o| o.origin).collect();
    assert!(origins.contains(&RuleOrigin::UserDefined));
    assert!(origins.contains(&RuleOrigin::SystemDefault));
    assert!(origins.contains(&RuleOrigin::CorrectionDerived));
}

#[test]
fn a_rule_below_the_prompt_score_floor_is_not_marked_in_prompt() {
    // Retiring a rule that occupies no slot changes nothing observable while
    // still spending a reviewer's decision.
    let graph = store();
    let engine = RulesEngine::new(&graph);
    let rule = engine
        .add_rule("a faded rule", RuleSource::CorrectionDerived)
        .expect("add");
    engine
        .apply_score_delta(&rule.id, -49.0, "test-fade")
        .expect("fade");

    let observations = rule_observations(&graph);

    let observed = observations
        .iter()
        .find(|observation| observation.rule_id == rule.id)
        .expect("observed");
    assert!(
        !observed.in_prompt,
        "a rule below the score floor is not in the prompt block"
    );
}

/// A store whose correction search always fails.
struct FailingSearch(MemoryGraph);

#[rustfmt::skip]
impl MemoryTrait for FailingSearch {
    fn store_memory(&self, content: &str, title: &str, memory_type: MemoryType, importance: f64, tags: &[String], source_type: &str, project_path: &str) -> Result<String, MemoryError> {
        self.0.store_memory(content, title, memory_type, importance, tags, source_type, project_path)
    }
    fn store_memory_with_id_outcome(&self, id: &str, content: &str, title: &str, memory_type: MemoryType, importance: f64, tags: &[String], source_type: &str, project_path: &str) -> Result<archon_memory::StoreMemoryOutcome, MemoryError> {
        self.0.store_memory_with_id_outcome(id, content, title, memory_type, importance, tags, source_type, project_path)
    }
    fn store_memory_with_id(&self, id: &str, content: &str, title: &str, memory_type: MemoryType, importance: f64, tags: &[String], source_type: &str, project_path: &str) -> Result<archon_memory::types::Memory, MemoryError> {
        self.0.store_memory_with_id(id, content, title, memory_type, importance, tags, source_type, project_path)
    }
    fn get_memory(&self, id: &str) -> Result<archon_memory::types::Memory, MemoryError> { self.0.get_memory(id) }
    fn inspect_memory(&self, id: &str) -> Result<archon_memory::types::Memory, MemoryError> { self.0.inspect_memory(id) }
    fn update_memory(&self, id: &str, content: Option<&str>, tags: Option<&[String]>) -> Result<(), MemoryError> { self.0.update_memory(id, content, tags) }
    fn apply_importance_delta(&self, id: &str, delta: f64, provenance_id: &str) -> Result<archon_memory::types::Memory, MemoryError> { self.0.apply_importance_delta(id, delta, provenance_id) }
    fn has_importance_application(&self, memory_id: &str, provenance_id: &str) -> Result<bool, MemoryError> { self.0.has_importance_application(memory_id, provenance_id) }
    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> { self.0.delete_memory(id) }
    fn create_relationship(&self, from_id: &str, to_id: &str, rel_type: archon_memory::RelType, context: Option<&str>, strength: f64) -> Result<(), MemoryError> { self.0.create_relationship(from_id, to_id, rel_type, context, strength) }
    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<archon_memory::types::Memory>, MemoryError> { self.0.recall_memories(query, limit) }
    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<archon_memory::types::Memory>, MemoryError> {
        if filter.memory_type == Some(MemoryType::Correction) {
            return Err(MemoryError::Database("correction search unavailable".into()));
        }
        self.0.search_memories(filter)
    }
    fn list_recent(&self, limit: usize) -> Result<Vec<archon_memory::types::Memory>, MemoryError> { self.0.list_recent(limit) }
    fn memory_count(&self) -> Result<usize, MemoryError> { self.0.memory_count() }
    fn clear_all(&self) -> Result<usize, MemoryError> { self.0.clear_all() }
    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<archon_memory::types::Memory>, MemoryError> { self.0.get_related_memories(id, depth) }
}

#[test]
fn unreadable_correction_evidence_keeps_the_rule_rather_than_reading_as_silence() {
    // THE assertion for this module. Absent corrections are what retires a
    // rule, so a failed read must not be indistinguishable from an empty one --
    // a database hiccup would otherwise propose retiring every rule at once.
    let graph = FailingSearch(store());
    let engine = RulesEngine::new(&graph);
    engine
        .add_rule("check constraints", RuleSource::CorrectionDerived)
        .expect("add");

    let observations = rule_observations(&graph);

    let observed = observations.first().expect("observed");
    assert!(
        observed.supporting_corrections > 0,
        "an unreadable correction search must not read as silence"
    );
    assert!(observed.most_recent_correction.is_some());

    // And the pure analysis must then decline to retire it.
    let candidates = archon_memory::garden::rule_retirement_candidates(
        &observations,
        &archon_memory::garden::RuleRetirementPolicy::default(),
        chrono::Utc::now(),
    );
    assert!(
        candidates.is_empty(),
        "a rule whose evidence could not be read was proposed for retirement"
    );
}

#[test]
fn an_unreadable_rule_list_proposes_nothing() {
    // The safe direction again: no rules observed means no rules retired.
    let graph: Arc<dyn MemoryTrait> = Arc::new(store());

    let observations = rule_observations(graph.as_ref());

    assert!(observations.is_empty());
}
