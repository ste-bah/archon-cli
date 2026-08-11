//! Tests for the semantic deduplication phase.
//!
//! An in-memory graph carries no embeddings, so `embedding_neighbours` reports
//! itself unavailable and the phase would skip whatever it was given. Most of
//! these use a delegating wrapper that supplies canned neighbours, so the merge
//! decisions are actually exercised rather than skipped.

use std::collections::HashMap;

use super::phases::phase_semantic_dedup;
use super::{BudgetLedger, GardenBudget};
use crate::access::MemoryTrait;
use crate::graph::MemoryGraph;
use crate::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter, StoreMemoryOutcome};

/// A ledger that never refuses, so these tests measure the dedup rules and not
/// the work budget. The budget has its own tests.
fn unbounded() -> BudgetLedger {
    BudgetLedger::new(GardenBudget::unbounded())
}

/// Wraps a real graph and answers `embedding_neighbours` from a fixed table.
struct StubNeighbours {
    inner: MemoryGraph,
    neighbours: HashMap<String, Vec<(String, f64)>>,
}

impl StubNeighbours {
    fn new(inner: MemoryGraph) -> Self {
        Self {
            inner,
            neighbours: HashMap::new(),
        }
    }

    /// Declare `a` and `b` as neighbours of each other at `distance`.
    fn link(&mut self, a: &str, b: &str, distance: f64) {
        self.neighbours
            .entry(a.to_string())
            .or_default()
            .push((b.to_string(), distance));
        self.neighbours
            .entry(b.to_string())
            .or_default()
            .push((a.to_string(), distance));
    }

    fn store(&self, content: &str, memory_type: MemoryType, importance: f64) -> String {
        self.inner
            .store_memory(content, "", memory_type, importance, &[], "test", "")
            .expect("store")
    }
}

#[rustfmt::skip]
impl MemoryTrait for StubNeighbours {
    fn store_memory(&self, content: &str, title: &str, memory_type: MemoryType, importance: f64, tags: &[String], source_type: &str, project_path: &str) -> Result<String, MemoryError> {
        self.inner.store_memory(content, title, memory_type, importance, tags, source_type, project_path)
    }
    fn store_memory_with_id_outcome(&self, id: &str, content: &str, title: &str, memory_type: MemoryType, importance: f64, tags: &[String], source_type: &str, project_path: &str) -> Result<StoreMemoryOutcome, MemoryError> {
        self.inner.store_memory_with_id_outcome(id, content, title, memory_type, importance, tags, source_type, project_path)
    }
    fn store_memory_with_id(&self, id: &str, content: &str, title: &str, memory_type: MemoryType, importance: f64, tags: &[String], source_type: &str, project_path: &str) -> Result<Memory, MemoryError> {
        self.inner.store_memory_with_id(id, content, title, memory_type, importance, tags, source_type, project_path)
    }
    fn get_memory(&self, id: &str) -> Result<Memory, MemoryError> { self.inner.get_memory(id) }
    fn inspect_memory(&self, id: &str) -> Result<Memory, MemoryError> { self.inner.inspect_memory(id) }
    fn update_memory(&self, id: &str, content: Option<&str>, tags: Option<&[String]>) -> Result<(), MemoryError> {
        self.inner.update_memory(id, content, tags)
    }
    fn apply_importance_delta(&self, id: &str, delta: f64, provenance_id: &str) -> Result<Memory, MemoryError> {
        self.inner.apply_importance_delta(id, delta, provenance_id)
    }
    fn has_importance_application(&self, memory_id: &str, provenance_id: &str) -> Result<bool, MemoryError> {
        self.inner.has_importance_application(memory_id, provenance_id)
    }
    fn delete_memory(&self, id: &str) -> Result<(), MemoryError> { self.inner.delete_memory(id) }
    fn create_relationship(&self, from_id: &str, to_id: &str, rel_type: RelType, context: Option<&str>, strength: f64) -> Result<(), MemoryError> {
        self.inner.create_relationship(from_id, to_id, rel_type, context, strength)
    }
    fn recall_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>, MemoryError> {
        self.inner.recall_memories(query, limit)
    }
    fn search_memories(&self, filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        self.inner.search_memories(filter)
    }
    fn list_recent(&self, limit: usize) -> Result<Vec<Memory>, MemoryError> { self.inner.list_recent(limit) }
    fn memory_count(&self) -> Result<usize, MemoryError> { self.inner.memory_count() }
    fn clear_all(&self) -> Result<usize, MemoryError> { self.inner.clear_all() }
    fn get_related_memories(&self, id: &str, depth: u32) -> Result<Vec<Memory>, MemoryError> {
        self.inner.get_related_memories(id, depth)
    }

    fn embedding_neighbours(&self, memory_id: &str, top_k: usize) -> Result<Option<Vec<(String, f64)>>, MemoryError> {
        // Always `Some`: this stub stands in for a store that HAS vector
        // search. Absence is exercised against a real unindexed graph below.
        Ok(Some(self.neighbours.get(memory_id).cloned().unwrap_or_default().into_iter().take(top_k).collect()))
    }
}

fn graph() -> MemoryGraph {
    MemoryGraph::in_memory().expect("graph")
}

/// The case this phase exists for: two memories stating one instruction in
/// different words, which the lexical pass cannot see.
#[test]
fn merges_a_restatement_that_lexical_overlap_would_miss() {
    let mut stub = StubNeighbours::new(graph());
    let a = stub.store(
        "Deploy region: eu-west-2 only, never us-east-1",
        MemoryType::Fact,
        0.6,
    );
    let b = stub.store(
        "The user requires all deploys to target eu-west-2 only",
        MemoryType::Fact,
        0.4,
    );
    stub.link(&a, &b, 0.04);

    let (merged, _linked) =
        phase_semantic_dedup(&stub, 0.15, 0.15, 50, &mut unbounded()).expect("dedup");

    assert_eq!(merged, Some(1));
    // Higher importance survives.
    assert!(
        stub.get_memory(&a).is_ok(),
        "the stronger memory must survive"
    );
    // Marked, not destroyed. The row stays reachable by id so the `Supersedes`
    // edge points at something real and a wrong merge can be undone -- that
    // reversibility is what lets the threshold be set from measurement rather
    // than from fear.
    let victim = stub
        .get_memory(&b)
        .expect("the restatement must still exist");
    assert!(
        crate::types::is_superseded(&victim.tags),
        "the folded memory must be marked superseded, got tags {:?}",
        victim.tags
    );
    // ...and invisible to anything that reads memories in bulk.
    assert!(
        !stub
            .list_recent(50)
            .expect("list")
            .iter()
            .any(|m| m.id == b),
        "a superseded memory must not appear in listings"
    );
}

/// Distance above the threshold must not merge.
///
/// This is the guard that matters most: merging deletes, and two memories about
/// the same subject are not necessarily the same claim.
#[test]
fn leaves_related_but_distinct_memories_alone() {
    let mut stub = StubNeighbours::new(graph());
    let a = stub.store("Deploy to eu-west-2", MemoryType::Fact, 0.6);
    let b = stub.store("Never deploy to us-east-1", MemoryType::Fact, 0.5);
    // Related enough to be neighbours, not close enough to be the same claim.
    stub.link(&a, &b, 0.31);

    let (merged, _linked) =
        phase_semantic_dedup(&stub, 0.15, 0.15, 50, &mut unbounded()).expect("dedup");

    assert_eq!(merged, Some(0));
    assert!(stub.get_memory(&a).is_ok());
    assert!(stub.get_memory(&b).is_ok());
}

/// The review band must not write anything to the graph.
///
/// It originally recorded a `RelatedTo` edge to mark the pair for later
/// adjudication. `phase_fragment_merge` runs immediately afterwards and selects
/// its candidates with `get_related_memories`, so every "probably related,
/// decide nothing" edge became a hard delete on the next phase -- 13 memories
/// destroyed on a real store by pairs this band had deliberately spared.
///
/// A band that exists to withhold a decision cannot leave behind anything
/// another phase reads as a decision.
#[test]
fn the_review_band_records_no_relationship() {
    let mut stub = StubNeighbours::new(graph());
    let a = stub.store("Deploy to eu-west-2", MemoryType::Fact, 0.6);
    let b = stub.store("Never deploy to us-east-1", MemoryType::Fact, 0.5);
    // Inside the review band: closer than "unrelated", further than "the same".
    stub.link(&a, &b, 0.25);

    let (merged, review) =
        phase_semantic_dedup(&stub, 0.15, 0.35, 50, &mut unbounded()).expect("dedup");

    assert_eq!(merged, Some(0), "the review band must never merge");
    assert_eq!(
        review.len(),
        1,
        "the pair must still be reported for adjudication"
    );
    assert!(
        stub.get_related_memories(&a, 1)
            .expect("related")
            .is_empty(),
        "the review band must not create relationships; a later phase treats \
         them as merge candidates and deletes"
    );
    assert!(stub.get_memory(&a).is_ok());
    assert!(stub.get_memory(&b).is_ok());
}

/// A `SameClaim` verdict merges through the same path as the automatic passes.
#[test]
fn adjudicated_same_claim_merges_and_supersedes() {
    let stub = StubNeighbours::new(graph());
    let a = stub.store("Deploy only to eu-west-2", MemoryType::Fact, 0.6);
    let b = stub.store("The deploy region is eu-west-2", MemoryType::Fact, 0.4);
    let pair = super::ReviewPair {
        a_id: a.clone(),
        b_id: b.clone(),
        a_content: "Deploy only to eu-west-2".into(),
        b_content: "The deploy region is eu-west-2".into(),
    };

    let merged = super::apply_adjudicated_merges(&stub, &[(pair, super::Adjudication::SameClaim)])
        .expect("apply");

    assert_eq!(merged, 1);
    let victim = stub.get_memory(&b).expect("victim retained");
    assert!(crate::types::is_superseded(&victim.tags));
    assert!(stub.get_memory(&a).is_ok());
}

/// A `Distinct` verdict must leave both memories alone.
#[test]
fn adjudicated_distinct_changes_nothing() {
    let stub = StubNeighbours::new(graph());
    let a = stub.store("Deploy to eu-west-2", MemoryType::Fact, 0.6);
    let b = stub.store("Never deploy to us-east-1", MemoryType::Fact, 0.5);
    let pair = super::ReviewPair {
        a_id: a.clone(),
        b_id: b.clone(),
        a_content: "Deploy to eu-west-2".into(),
        b_content: "Never deploy to us-east-1".into(),
    };

    let merged = super::apply_adjudicated_merges(&stub, &[(pair, super::Adjudication::Distinct)])
        .expect("apply");

    assert_eq!(merged, 0);
    let a_row = stub.get_memory(&a).expect("a");
    let b_row = stub.get_memory(&b).expect("b");
    assert!(!crate::types::is_superseded(&a_row.tags));
    assert!(!crate::types::is_superseded(&b_row.tags));
}

/// Verdicts are formed against a snapshot, and an adjudicator round-trip is slow.
///
/// If either memory was superseded in the meantime, applying the verdict would
/// fold an already-dead row into a live one and drag its tags back with it.
#[test]
fn a_stale_verdict_on_an_already_superseded_memory_is_skipped() {
    let stub = StubNeighbours::new(graph());
    let a = stub.store("Deploy only to eu-west-2", MemoryType::Fact, 0.6);
    let b = stub.store("The deploy region is eu-west-2", MemoryType::Fact, 0.4);
    // Something else folded `b` away while the verdict was in flight.
    let mut tags = stub.get_memory(&b).expect("b").tags;
    tags.push(crate::types::SUPERSEDED_TAG.to_string());
    stub.update_memory(&b, None, Some(&tags))
        .expect("supersede");

    let pair = super::ReviewPair {
        a_id: a.clone(),
        b_id: b.clone(),
        a_content: "Deploy only to eu-west-2".into(),
        b_content: "The deploy region is eu-west-2".into(),
    };
    let merged = super::apply_adjudicated_merges(&stub, &[(pair, super::Adjudication::SameClaim)])
        .expect("apply");

    assert_eq!(merged, 0, "a stale verdict must not be applied");
}

/// A verdict naming a memory that no longer exists is skipped, not fatal.
#[test]
fn a_verdict_for_a_missing_memory_does_not_fail_the_batch() {
    let stub = StubNeighbours::new(graph());
    let a = stub.store("still here", MemoryType::Fact, 0.6);
    let pair = super::ReviewPair {
        a_id: a.clone(),
        b_id: "no-such-memory".into(),
        a_content: "still here".into(),
        b_content: "gone".into(),
    };

    let merged = super::apply_adjudicated_merges(&stub, &[(pair, super::Adjudication::SameClaim)])
        .expect("a missing memory must not fail the batch");

    assert_eq!(merged, 0);
    assert!(stub.get_memory(&a).is_ok());
}

/// A store that HAS vector search and finds nothing near reports zero merges.
///
/// The counterpart to the test below, and the guard against fixing the
/// unavailable case by inverting it: "nothing to merge" must stay sayable.
#[test]
fn a_searchable_store_with_no_near_neighbours_reports_zero() {
    let stub = StubNeighbours::new(graph());
    stub.store("something", MemoryType::Fact, 0.5);
    stub.store("something else", MemoryType::Fact, 0.5);

    assert_eq!(
        phase_semantic_dedup(&stub, 0.15, 0.15, 50, &mut unbounded())
            .expect("dedup")
            .0,
        Some(0)
    );
}

/// A store with no vector index reports the pass as UNAVAILABLE, not clean.
///
/// The distinction the `Option` exists for. A plain in-memory graph never
/// initialised `memory_embeddings`, so there is no index to query -- the same
/// condition as a second Archon process reading memory over TCP. Reporting
/// `Some(0)` here is the bug: it says the store was examined and found to hold
/// no duplicates, when nothing was examined at all.
#[test]
fn an_unindexed_store_reports_the_pass_as_unavailable() {
    let graph = graph();
    graph
        .store_memory("something", "", MemoryType::Fact, 0.5, &[], "test", "")
        .expect("store");

    let (merged, review) =
        phase_semantic_dedup(&graph, 0.15, 0.35, 50, &mut unbounded()).expect("dedup");

    assert_eq!(merged, None, "an absent index is not a clean store");
    assert!(review.is_empty());
}

/// The budget stops a single pass reshaping the whole graph.
#[test]
fn respects_the_merge_budget() {
    let mut stub = StubNeighbours::new(graph());
    for i in 0..6 {
        let a = stub.store(&format!("pair {i} original"), MemoryType::Fact, 0.6);
        let b = stub.store(&format!("pair {i} restated"), MemoryType::Fact, 0.4);
        stub.link(&a, &b, 0.01);
    }

    let (merged, _linked) =
        phase_semantic_dedup(&stub, 0.15, 0.15, 2, &mut unbounded()).expect("dedup");

    assert_eq!(merged, Some(2), "the pass must stop at its budget");
}

/// End-to-end against a REAL vector index, not the stub.
///
/// Everything above uses canned neighbours, which proves the merge decisions
/// and nothing about `embedding_neighbours` itself -- the stored-vector read and
/// the Cozo HNSW query are bypassed entirely. This exercises both, and pins the
/// assumption the whole threshold rests on: that the index returns cosine
/// DISTANCE, where 0 is identical. If it ever returned a similarity instead,
/// every comparison would invert and 0.08 would merge unrelated memories.
#[test]
fn real_vector_index_merges_near_neighbours_and_spares_distant_ones() {
    let graph = MemoryGraph::in_memory().expect("graph");
    crate::vector_search::init_embedding_schema(graph.db(), 4).expect("embedding schema");

    let store = |content: &str, importance: f64| {
        graph
            .store_memory(content, "", MemoryType::Fact, importance, &[], "test", "")
            .expect("store")
    };

    let anchor = store("deploy to eu-west-2", 0.9);
    let paraphrase = store("target the eu-west-2 region", 0.4);
    let unrelated = store("python is good for data science", 0.5);

    // Hand-built so the geometry is exact rather than model-dependent.
    let put = |id: &str, v: [f32; 4]| {
        crate::vector_search::store_embedding(graph.db(), id, &v, "test", 4).expect("embedding")
    };
    put(&anchor, [1.0, 0.0, 0.0, 0.0]);
    // ~0.996 cosine similarity with the anchor: a paraphrase.
    put(&paraphrase, [0.99, 0.09, 0.0, 0.0]);
    // Orthogonal: nothing to do with it.
    put(&unrelated, [0.0, 1.0, 0.0, 0.0]);

    // Distance semantics, asserted directly.
    let neighbours = graph
        .embedding_neighbours(&anchor, 8)
        .expect("neighbour search")
        .expect("a store with a live index reports available");
    let distance_to = |id: &str| {
        neighbours
            .iter()
            .find(|(nid, _)| nid == id)
            .map(|(_, d)| *d)
            .unwrap_or_else(|| panic!("{id} missing from neighbours: {neighbours:?}"))
    };
    assert!(
        distance_to(&paraphrase) < 0.08,
        "a near-identical vector must be under the merge threshold, got {}",
        distance_to(&paraphrase)
    );
    assert!(
        distance_to(&unrelated) > 0.5,
        "an orthogonal vector must be far, got {} -- if this is small the index \
         is returning similarity, not distance, and the threshold is inverted",
        distance_to(&unrelated)
    );

    let (merged, _linked) =
        phase_semantic_dedup(&graph, 0.15, 0.15, 50, &mut unbounded()).expect("dedup");

    assert_eq!(merged, Some(1), "only the paraphrase should merge");
    assert!(graph.get_memory(&anchor).is_ok(), "the anchor survives");
    let folded = graph
        .get_memory(&paraphrase)
        .expect("the paraphrase is retained, not deleted");
    assert!(crate::types::is_superseded(&folded.tags));
    assert!(
        graph.get_memory(&unrelated).is_ok(),
        "the unrelated memory is untouched"
    );
}

/// A neighbour of a different type is not merged.
///
/// Rules are rendered into the system prompt and facts are not, so wording
/// similarity between them does not make them interchangeable.
#[test]
fn does_not_merge_across_memory_types() {
    let mut stub = StubNeighbours::new(graph());
    let fact = stub.store("Always deploy to eu-west-2", MemoryType::Fact, 0.6);
    let rule = stub.store("Always deploy to eu-west-2", MemoryType::Rule, 0.5);
    stub.link(&fact, &rule, 0.0);

    let (merged, _linked) =
        phase_semantic_dedup(&stub, 0.15, 0.15, 50, &mut unbounded()).expect("dedup");

    assert_eq!(merged, Some(0));
    assert!(stub.get_memory(&fact).is_ok());
    assert!(stub.get_memory(&rule).is_ok());
}
