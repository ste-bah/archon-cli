//! Filed-answer provenance persistence tests.

use std::{
    collections::BTreeMap,
    sync::{Arc, Barrier, Mutex, MutexGuard, OnceLock},
};

use cozo::{DataValue, ScriptMutability};

use super::*;
use crate::kb::schema::ensure_kb_schema;

fn test_db() -> cozo::DbInstance {
    let db = cozo::DbInstance::new("mem", "", Default::default()).unwrap();
    ensure_kb_schema(&db).unwrap();
    db
}

fn answer(text: &str) -> SynthesizedAnswer {
    SynthesizedAnswer {
        answer_text: text.to_string(),
        source_citations: vec![],
    }
}

fn provenance_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn provenance_targets(db: &cozo::DbInstance, owner: &str) -> Vec<String> {
    provenance_edges(db, owner)
        .into_iter()
        .map(|(_, target)| target)
        .collect()
}

fn provenance_edges(db: &cozo::DbInstance, owner: &str) -> Vec<(String, String)> {
    let mut params = BTreeMap::new();
    params.insert("nid".into(), DataValue::from(owner));
    let result = db
        .run_script(
            "?[edge_id, target_node_id] := *kb_edges{edge_id, source_node_id, target_node_id, edge_type}, \
             source_node_id = $nid, edge_type = 'DerivedFrom'",
            params,
            ScriptMutability::Immutable,
        )
        .unwrap();
    let mut edges: Vec<(String, String)> = result
        .rows
        .iter()
        .map(|row| {
            (
                row[0].get_str().unwrap().to_string(),
                row[1].get_str().unwrap().to_string(),
            )
        })
        .collect();
    edges.sort_by(|left, right| left.1.cmp(&right.1));
    edges
}

#[test]
fn multiple_sources_use_one_batch_write_and_persist_every_edge() {
    let _lock = provenance_test_lock();
    let db = test_db();
    let engine = QueryEngine::new(db.clone());
    super::super::provenance_storage::test_support::reset();

    let owner = engine
        .file_answer(
            "Question",
            &answer("Batch every source."),
            &["source-a".into(), "source-b".into(), "source-c".into()],
        )
        .unwrap();

    assert_eq!(
        super::super::provenance_storage::test_support::counts().write_queries,
        1
    );
    assert_eq!(
        provenance_targets(&db, &owner),
        vec!["source-a", "source-b", "source-c"]
    );
}

#[test]
fn empty_sources_skip_provenance_writes() {
    let _lock = provenance_test_lock();
    let db = test_db();
    let engine = QueryEngine::new(db);
    super::super::provenance_storage::test_support::reset();

    engine
        .file_answer("Question", &answer("No sources."), &[])
        .unwrap();

    assert_eq!(
        super::super::provenance_storage::test_support::counts().write_queries,
        0
    );
}

#[test]
fn repeated_sources_are_deduplicated_before_the_batch_write() {
    let _lock = provenance_test_lock();
    let db = test_db();
    let engine = QueryEngine::new(db.clone());
    super::super::provenance_storage::test_support::reset();

    let owner = engine
        .file_answer(
            "Question",
            &answer("Deduplicate sources."),
            &["source-a".into(), "source-a".into(), "source-b".into()],
        )
        .unwrap();

    assert_eq!(
        super::super::provenance_storage::test_support::counts().write_queries,
        1
    );
    assert_eq!(
        provenance_targets(&db, &owner),
        vec!["source-a", "source-b"]
    );
}

#[test]
fn reused_answer_owner_receives_only_new_provenance() {
    let _lock = provenance_test_lock();
    let db = test_db();
    let engine = QueryEngine::new(db.clone());
    let answer = answer("Reuse with provenance.");
    let owner = engine
        .file_answer("First", &answer, &["source-a".into()])
        .unwrap();
    super::super::provenance_storage::test_support::reset();

    let reused = engine
        .file_answer("Second", &answer, &["source-a".into(), "source-b".into()])
        .unwrap();

    assert_eq!(reused, owner);
    assert_eq!(
        super::super::provenance_storage::test_support::counts().write_queries,
        1
    );
    assert_eq!(
        provenance_targets(&db, &owner),
        vec!["source-a", "source-b"]
    );
}

#[test]
fn all_preexisting_sources_skip_provenance_writes() {
    let _lock = provenance_test_lock();
    let db = test_db();
    let engine = QueryEngine::new(db);
    let answer = answer("Reuse every provenance source.");
    engine
        .file_answer("First", &answer, &["source-a".into(), "source-b".into()])
        .unwrap();
    super::super::provenance_storage::test_support::reset();

    engine
        .file_answer("Second", &answer, &["source-a".into(), "source-b".into()])
        .unwrap();

    assert_eq!(
        super::super::provenance_storage::test_support::counts().write_queries,
        0
    );
}

#[test]
fn same_owner_and_source_concurrently_create_one_deterministic_edge() {
    let _lock = provenance_test_lock();
    let db = test_db();
    let owner = "answer-concurrent";
    let sources = vec!["source-a".to_string()];
    let start = Arc::new(Barrier::new(2));
    let first_db = db.clone();
    let first_start = Arc::clone(&start);
    let first_sources = sources.clone();
    let first = std::thread::spawn(move || {
        first_start.wait();
        super::super::provenance_storage::persist_derived_from_edges(
            &first_db,
            owner,
            &first_sources,
            1.0,
        )
    });
    let second_db = db.clone();
    let second_start = Arc::clone(&start);
    let second = std::thread::spawn(move || {
        second_start.wait();
        super::super::provenance_storage::persist_derived_from_edges(
            &second_db, owner, &sources, 2.0,
        )
    });

    first.join().unwrap().unwrap();
    second.join().unwrap().unwrap();

    assert_eq!(
        provenance_edges(&db, owner),
        vec![(
            "edge-cd2ada77b3d07995554623d929a1a686d66b26ab380d8a695c1997c879a80ed6".into(),
            "source-a".into(),
        )]
    );
}
#[test]
fn batch_failure_keeps_filed_answer_without_partial_edges() {
    let _lock = provenance_test_lock();
    let db = test_db();
    let engine = QueryEngine::new(db.clone());
    super::super::provenance_storage::test_support::reset();
    super::super::provenance_storage::test_support::fail_after_batch_write();

    let owner = engine
        .file_answer(
            "Question",
            &answer("Batch failure remains best effort."),
            &["source-a".into(), "source-b".into()],
        )
        .unwrap();

    assert_eq!(
        super::super::provenance_storage::test_support::counts().write_queries,
        1
    );
    assert_eq!(
        super::super::provenance_storage::test_support::counts().post_batch_writes,
        1
    );
    assert!(provenance_targets(&db, &owner).is_empty());
    let mut params = BTreeMap::new();
    params.insert("nid".into(), DataValue::from(owner.as_str()));
    let filed = db
        .run_script(
            "?[node_id] := *kb_nodes{node_id}, node_id = $nid",
            params,
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert_eq!(filed.rows.len(), 1);
}
