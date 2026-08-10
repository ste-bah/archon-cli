//! R7 end to end: a real CozoDB-backed knowledge adapter merged with three
//! ported stores, exercised through the public facade.
//!
//! No network, no model, no sleep except the one the latency budget test needs.
//! The knowledge source is the genuine article — a real Cozo instance, a real
//! docs schema, real chunk rows, and `hybrid_retriever` doing the retrieval —
//! so the mapping from `KnowledgeSearchResult` to `RecallHit` is covered by a
//! separate read rather than by a stub agreeing with itself.

use std::sync::Arc;
use std::time::Duration;

use archon_docs::models::ChunkArtifact;
use archon_knowledge::errors::{KnowledgeError, Result};
use archon_knowledge::hybrid_retriever::{SearchMode, SearchOptions};
use archon_knowledge::recall::adapters::{
    CodeIndexAdapter, DocsAdapter, KnowledgeStoreAdapter, MemoryAdapter, StoreRecord,
    StoreRecordSource,
};
use archon_knowledge::recall::{
    ConflictKind, RecallQuery, RecallSource, SourceBudget, SourcePolicy, SourceStatus,
    UnifiedRecall,
};
use archon_knowledge::schema;
use cozo::DbInstance;

fn db_with_chunks(chunks: &[(&str, &str, &str)]) -> Arc<DbInstance> {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_docs::schema::ensure_doc_schema(&db).unwrap();
    schema::ensure_knowledge_schema(&db).unwrap();
    for (chunk_id, document_id, content) in chunks {
        archon_docs::store::insert_chunk(
            &db,
            &ChunkArtifact {
                chunk_id: (*chunk_id).into(),
                document_id: (*document_id).into(),
                artifact_id: format!("artifact-{chunk_id}"),
                chunk_index: 0,
                page_start: 1,
                page_end: 1,
                content: (*content).into(),
                content_hash: format!("hash-{chunk_id}"),
                embedding_status: "pending".into(),
            },
        )
        .unwrap();
    }
    Arc::new(db)
}

fn knowledge_adapter(db: Arc<DbInstance>) -> Arc<KnowledgeStoreAdapter> {
    Arc::new(KnowledgeStoreAdapter::new(
        db,
        SearchOptions {
            mode: SearchMode::Exact,
            ..Default::default()
        },
    ))
}

/// A store port that answers from a fixed list.
struct Canned(Vec<StoreRecord>);

impl StoreRecordSource for Canned {
    fn search(&self, _text: &str, limit: usize) -> Result<Vec<StoreRecord>> {
        Ok(self.0.iter().take(limit).cloned().collect())
    }
}

struct Offline(&'static str);

impl StoreRecordSource for Offline {
    fn search(&self, _text: &str, _limit: usize) -> Result<Vec<StoreRecord>> {
        Err(KnowledgeError::Store(self.0.into()))
    }
}

struct Sleeper(Duration);

impl StoreRecordSource for Sleeper {
    fn search(&self, _text: &str, _limit: usize) -> Result<Vec<StoreRecord>> {
        std::thread::sleep(self.0);
        Ok(vec![StoreRecord::new("late", "late")])
    }
}

fn policy(entries: &[(RecallSource, usize, u64)]) -> SourcePolicy {
    SourcePolicy::from_budgets(
        entries
            .iter()
            .map(|&(source, quota, millis)| SourceBudget {
                source,
                quota,
                latency_budget: Duration::from_millis(millis),
            })
            .collect(),
    )
}

fn query(text: &str, limit: usize, entries: &[(RecallSource, usize, u64)]) -> RecallQuery {
    RecallQuery {
        text: text.into(),
        limit,
        source_policy: policy(entries),
    }
}

fn status(response: &archon_knowledge::recall::RecallResponse, source: RecallSource) -> String {
    let outcome = response
        .sources
        .iter()
        .find(|outcome| outcome.source == source)
        .unwrap_or_else(|| panic!("{source} missing from the accounting"));
    format!("{:?}", outcome.status)
}

#[test]
fn four_sources_merge_into_one_answer_with_full_accounting() {
    let db = db_with_chunks(&[("c1", "doc-1", "Retention of audit logs is thirty days.")]);
    let response = UnifiedRecall::new()
        .with_source(knowledge_adapter(db))
        .with_source(Arc::new(DocsAdapter::new(Arc::new(Canned(vec![
            StoreRecord::new("c2", "Audit logs are written on every action.")
                .with_container("doc-1"),
        ])))))
        .with_source(Arc::new(MemoryAdapter::new(Arc::new(Canned(vec![
            StoreRecord::new("mem-1", "We agreed to keep audit logs."),
        ])))))
        .with_source(Arc::new(CodeIndexAdapter::new(Arc::new(Canned(vec![
            StoreRecord::new("src/audit.rs:1-20", "fn write_audit_log() {}")
                .with_container("src/audit.rs"),
        ])))))
        .recall(&query(
            "audit logs retention",
            10,
            &[
                (RecallSource::Knowledge, 5, 30_000),
                (RecallSource::Docs, 5, 30_000),
                (RecallSource::Memory, 5, 30_000),
                (RecallSource::Code, 5, 30_000),
            ],
        ));

    assert_eq!(response.sources.len(), 4, "every source must be accounted");
    assert!(
        response
            .sources
            .iter()
            .all(|outcome| outcome.status.is_ok()),
        "{:?}",
        response.sources
    );
    assert!(!response.is_partial());
    assert_eq!(response.hits.len(), 4);

    // The real Cozo-backed source contributed, with the store's own score kept
    // but flagged uncalibrated.
    let knowledge = response
        .hits
        .iter()
        .find(|hit| hit.source == RecallSource::Knowledge)
        .expect("knowledge source returned nothing");
    assert_eq!(knowledge.source_id, "c1");
    assert!(knowledge.source_score.is_some());
    assert!(!knowledge.calibration.is_measured());
    assert_eq!(
        knowledge.provenance_refs,
        vec!["chunk:c1".to_string(), "doc:doc-1".to_string()]
    );
    assert!(response.calibration_note.contains("uncalibrated"));
}

/// The same chunk reached through docs and through the knowledge graph is one
/// artifact. Nothing shares a database here — only the provenance vocabulary.
#[test]
fn one_chunk_reached_through_two_stores_folds_into_a_single_hit() {
    let db = db_with_chunks(&[("c1", "doc-1", "Retention of audit logs is thirty days.")]);
    let response = UnifiedRecall::new()
        .with_source(knowledge_adapter(db))
        .with_source(Arc::new(DocsAdapter::new(Arc::new(Canned(vec![
            StoreRecord::new("c1", "Retention of audit logs is thirty days.")
                .with_container("doc-1"),
        ])))))
        .recall(&query(
            "audit logs retention",
            10,
            &[
                (RecallSource::Knowledge, 5, 30_000),
                (RecallSource::Docs, 5, 30_000),
            ],
        ));

    assert_eq!(response.hits.len(), 1, "{:?}", response.hits);
    assert_eq!(response.hits[0].duplicates.len(), 1);
    assert!(response.conflicts.is_empty());
}

#[test]
fn two_stores_disagreeing_about_one_chunk_surface_the_conflict() {
    let db = db_with_chunks(&[("c1", "doc-1", "Retention of audit logs is thirty days.")]);
    let response = UnifiedRecall::new()
        .with_source(knowledge_adapter(db))
        .with_source(Arc::new(DocsAdapter::new(Arc::new(Canned(vec![
            // Same chunk id, stale text: exactly the case that must not be
            // silently resolved by picking the higher score.
            StoreRecord::new("c1", "Retention of audit logs is ninety days.")
                .with_container("doc-1"),
        ])))))
        .recall(&query(
            "audit logs retention",
            10,
            &[
                (RecallSource::Knowledge, 5, 30_000),
                (RecallSource::Docs, 5, 30_000),
            ],
        ));

    assert_eq!(response.hits.len(), 2, "a conflict was deduped away");
    let divergent: Vec<_> = response
        .conflicts
        .iter()
        .filter(|conflict| conflict.kind == ConflictKind::DivergentContentForProvenance)
        .collect();
    assert!(!divergent.is_empty(), "{:?}", response.conflicts);
    assert!(response.hits.iter().all(|hit| !hit.conflicts.is_empty()));
}

#[test]
fn a_failing_source_leaves_the_others_answering() {
    let db = db_with_chunks(&[("c1", "doc-1", "Retention of audit logs is thirty days.")]);
    let response = UnifiedRecall::new()
        .with_source(knowledge_adapter(db))
        .with_source(Arc::new(CodeIndexAdapter::new(Arc::new(Offline(
            "no code index at /nowhere",
        )))))
        .recall(&query(
            "audit logs retention",
            10,
            &[
                (RecallSource::Knowledge, 5, 30_000),
                (RecallSource::Code, 5, 30_000),
            ],
        ));

    assert_eq!(response.hits.len(), 1);
    assert!(response.is_partial());
    assert!(
        status(&response, RecallSource::Code).contains("no code index at /nowhere"),
        "{}",
        status(&response, RecallSource::Code)
    );
}

#[test]
fn a_source_past_its_budget_is_abandoned_not_waited_on() {
    let db = db_with_chunks(&[("c1", "doc-1", "Retention of audit logs is thirty days.")]);
    let started = std::time::Instant::now();
    let response = UnifiedRecall::new()
        .with_source(knowledge_adapter(db))
        .with_source(Arc::new(MemoryAdapter::new(Arc::new(Sleeper(
            Duration::from_secs(30),
        )))))
        .recall(&query(
            "audit logs retention",
            10,
            &[
                (RecallSource::Knowledge, 5, 30_000),
                (RecallSource::Memory, 5, 25),
            ],
        ));
    let elapsed = started.elapsed();

    assert!(
        elapsed < Duration::from_secs(5),
        "waited {elapsed:?} on a 25ms budget"
    );
    assert_eq!(response.hits.len(), 1);
    assert!(matches!(
        response
            .sources
            .iter()
            .find(|outcome| outcome.source == RecallSource::Memory)
            .map(|outcome| &outcome.status),
        Some(SourceStatus::LatencyBudgetExceeded { .. })
    ));
}

#[test]
fn quotas_stop_one_store_from_filling_the_answer() {
    let bulk: Vec<StoreRecord> = (0..20)
        .map(|index| StoreRecord::new(format!("mem-{index}"), format!("memory note {index}")))
        .collect();
    let db = db_with_chunks(&[("c1", "doc-1", "Retention of audit logs is thirty days.")]);

    let response = UnifiedRecall::new()
        .with_source(knowledge_adapter(db))
        .with_source(Arc::new(MemoryAdapter::new(Arc::new(Canned(bulk)))))
        .recall(&query(
            "audit logs retention",
            10,
            &[
                (RecallSource::Knowledge, 5, 30_000),
                (RecallSource::Memory, 2, 30_000),
            ],
        ));

    let memory_hits = response
        .hits
        .iter()
        .filter(|hit| hit.source == RecallSource::Memory)
        .count();
    assert_eq!(memory_hits, 2, "memory exceeded its quota");
    assert_eq!(response.hits.len(), 3);
}

/// Two replays of one query over one fixture must produce one ordering.
#[test]
fn the_merged_order_is_stable_across_replays() {
    let build = || {
        let db = db_with_chunks(&[
            ("c1", "doc-1", "Retention of audit logs is thirty days."),
            ("c2", "doc-2", "Audit retention policy lives here."),
        ]);
        UnifiedRecall::new()
            .with_source(knowledge_adapter(db))
            .with_source(Arc::new(MemoryAdapter::new(Arc::new(Canned(vec![
                StoreRecord::new("mem-1", "audit retention was agreed"),
                StoreRecord::new("mem-2", "audit retention owner is platform"),
            ])))))
            .with_source(Arc::new(CodeIndexAdapter::new(Arc::new(Canned(vec![
                StoreRecord::new("src/audit.rs:1-20", "fn audit_retention() {}")
                    .with_container("src/audit.rs"),
            ])))))
            .recall(&query(
                "audit retention",
                10,
                &[
                    (RecallSource::Knowledge, 4, 30_000),
                    (RecallSource::Memory, 4, 30_000),
                    (RecallSource::Code, 4, 30_000),
                ],
            ))
    };

    let ids = |response: &archon_knowledge::recall::RecallResponse| -> Vec<String> {
        response
            .hits
            .iter()
            .map(|hit| format!("{}/{}", hit.source, hit.source_id))
            .collect()
    };
    assert_eq!(ids(&build()), ids(&build()));
}
