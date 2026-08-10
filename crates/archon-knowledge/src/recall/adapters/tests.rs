use super::*;
use crate::errors::KnowledgeError;
use crate::recall::{RecallQuery, SourceBudget, SourcePolicy};

struct Canned {
    records: Vec<StoreRecord>,
    /// Records the limit the adapter passed down, so the quota can be pinned.
    seen_limit: std::sync::Mutex<Option<usize>>,
}

impl Canned {
    fn new(records: Vec<StoreRecord>) -> Arc<Self> {
        Arc::new(Self {
            records,
            seen_limit: std::sync::Mutex::new(None),
        })
    }
}

impl StoreRecordSource for Canned {
    fn search(&self, _text: &str, limit: usize) -> Result<Vec<StoreRecord>> {
        *self.seen_limit.lock().expect("limit lock") = Some(limit);
        Ok(self.records.clone())
    }
}

struct Broken;

impl StoreRecordSource for Broken {
    fn search(&self, _text: &str, _limit: usize) -> Result<Vec<StoreRecord>> {
        Err(KnowledgeError::Store("store is offline".into()))
    }
}

fn query_for(source: RecallSource, quota: usize) -> RecallQuery {
    RecallQuery {
        text: "retention".into(),
        limit: quota,
        source_policy: SourcePolicy::from_budgets(vec![SourceBudget {
            source,
            quota,
            latency_budget: std::time::Duration::from_secs(1),
        }]),
    }
}

#[test]
fn docs_and_knowledge_agree_on_the_provenance_vocabulary() {
    // Load-bearing: this shared spelling is the only thing that lets the merge
    // recognise one chunk reached through two stores.
    assert_eq!(
        chunk_refs("chunk-1", Some("doc-1")),
        vec!["chunk:chunk-1", "doc:doc-1"]
    );
    assert_eq!(chunk_refs("chunk-1", None), vec!["chunk:chunk-1"]);
}

#[test]
fn docs_adapter_maps_records_to_ranked_hits() {
    let store = Canned::new(vec![
        StoreRecord::new("chunk-a", "alpha")
            .with_container("doc-1")
            .with_score(0.9),
        StoreRecord::new("chunk-b", "beta").with_container("doc-1"),
    ]);
    let hits = DocsAdapter::new(store.clone())
        .recall(&query_for(RecallSource::Docs, 4))
        .expect("docs recall");

    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].source, RecallSource::Docs);
    assert_eq!(hits[0].source_rank, 0);
    assert_eq!(hits[1].source_rank, 1);
    assert_eq!(hits[0].source_score, Some(0.9));
    assert_eq!(hits[1].source_score, None);
    assert_eq!(hits[0].provenance_refs, vec!["chunk:chunk-a", "doc:doc-1"]);
    // The quota, not the caller's raw limit, is what reaches the store.
    assert_eq!(*store.seen_limit.lock().unwrap(), Some(4));
}

/// A memory's project path is a scope, not an artifact. Emitting it as
/// provenance would make every memory in one project look like one artifact.
#[test]
fn memory_adapter_does_not_emit_its_container_as_provenance() {
    let store = Canned::new(vec![
        StoreRecord::new("mem-1", "first").with_container("F:/repo"),
        StoreRecord::new("mem-2", "second").with_container("F:/repo"),
    ]);
    let hits = MemoryAdapter::new(store)
        .recall(&query_for(RecallSource::Memory, 4))
        .expect("memory recall");

    assert_eq!(hits[0].provenance_refs, vec!["memory:mem-1"]);
    assert_eq!(hits[1].provenance_refs, vec!["memory:mem-2"]);
}

#[test]
fn code_adapter_normalises_path_separators() {
    let store = Canned::new(vec![
        StoreRecord::new("src\\lib.rs:10-20", "fn main() {}").with_container("src\\lib.rs"),
    ]);
    let hits = CodeIndexAdapter::new(store)
        .recall(&query_for(RecallSource::Code, 4))
        .expect("code recall");

    assert_eq!(hits[0].provenance_refs, vec!["file:src/lib.rs"]);
}

#[test]
fn a_store_error_reaches_the_caller_unchanged() {
    let error = DocsAdapter::new(Arc::new(Broken))
        .recall(&query_for(RecallSource::Docs, 4))
        .expect_err("expected the store error");
    assert!(error.to_string().contains("store is offline"));
}

#[test]
fn an_excluded_source_asks_its_store_for_nothing() {
    let store = Canned::new(vec![StoreRecord::new("chunk-a", "alpha")]);
    // Policy names only Memory, so the docs quota is zero.
    let _ = DocsAdapter::new(store.clone()).recall(&query_for(RecallSource::Memory, 4));
    assert_eq!(*store.seen_limit.lock().unwrap(), Some(0));
}
