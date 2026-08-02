use std::ffi::OsString;
use std::time::{Duration, Instant};

use crate::embed::LocalEmbeddingProvider;
use crate::errors::DocsError;
use crate::indexing::{IndexOptions, index_loaded_chunks_with_options_progress};
use crate::models::ChunkArtifact;
use crate::retrieval::{RetrievalWeights, SearchMode, search_with_mode};
use crate::vector_store::DocVectorStore;

const PROVIDER: &str = "runtime-evidence";

struct EvidenceProvider;

impl LocalEmbeddingProvider for EvidenceProvider {
    fn embed_chunks(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, DocsError> {
        Ok(chunks.iter().map(|chunk| vector_for(chunk)).collect())
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, DocsError> {
        Ok(vector_for(query))
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        PROVIDER
    }
}

#[test]
#[serial_test::serial(docs_global_state)]
fn persisted_runtime_evidence_covers_docs_acceptance_findings() {
    let _provider = crate::embed::install_provider_for_test(Box::new(EvidenceProvider));
    let _hnsw = crate::vector_store::hnsw_state_guard();
    let fixture = Fixture::new();
    let source = fixture.persist_source_of_truth();
    source.assert_invariants();
    let query = fixture.query_persisted_corpus();
    query.assert_invariants();
    source.print();
    query.print();
}

struct Fixture {
    temp: tempfile::TempDir,
    _vector_env: EnvGuard,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("create persisted evidence fixture");
        let vector_env = EnvGuard::set("ARCHON_DOC_VECTOR_STORE_DIR", temp.path().join("vectors"));
        Self {
            temp,
            _vector_env: vector_env,
        }
    }

    fn persist_source_of_truth(&self) -> SourceEvidence {
        let db_path = self.temp.path().join("docs.sqlite");
        let db = crate::acquire_docs_db(&db_path).expect("open persisted fixture database");
        let chunks = fixture_chunks();
        for chunk in &chunks {
            crate::store::insert_chunk(&db, chunk).expect("persist fixture chunk");
        }
        crate::schema::ensure_doc_schema(&db).expect("backfill FTS schema");
        let started = Instant::now();
        let mut completed = false;
        let indexed = index_loaded_chunks_with_options_progress(
            &db,
            chunks,
            &IndexOptions {
                batch_size: 2,
                embedding_workers: Some(1),
                ..Default::default()
            },
            |event| completed |= event.phase == crate::indexing::IndexProgressPhase::Complete,
        )
        .expect("index persisted evidence fixture");
        assert!(completed, "index completion event must be published");
        let vectors = DocVectorStore::acquire_default().expect("acquire vector source of truth");
        let manifest = vectors
            .latest_hnsw_manifest(PROVIDER)
            .expect("read HNSW manifest")
            .expect("snapshot published after ingest");
        let stats = vectors
            .stats(Some(PROVIDER))
            .expect("count vector prefixes");
        let handles = canonical_handle_evidence(self.temp.path());
        drop(db);
        SourceEvidence {
            db_path,
            vector_count: stats.raw_vectors,
            cache_count: stats.cache_entries,
            manifest_generation: manifest.provider_generation,
            manifest_count: manifest.vector_count,
            indexed_count: indexed.indexed,
            elapsed: started.elapsed(),
            canonical_handles_reused: handles,
        }
    }

    fn query_persisted_corpus(&self) -> QueryEvidence {
        let db = crate::open_docs_db_for_test(self.temp.path().join("docs.sqlite"))
            .expect("open persisted fixture database without cache or migration");
        let vectors = DocVectorStore::acquire_default().expect("acquire persisted vector store");
        let manifest = vectors
            .latest_hnsw_manifest(PROVIDER)
            .expect("read persisted manifest")
            .expect("persisted HNSW manifest");
        let manifest_files = hnsw_manifest_files(self.temp.path(), &manifest);
        crate::vector_store::clear_persisted_hnsw_cache();
        let loads_before = crate::vector_store::persisted_hnsw_load_count();
        let started = Instant::now();
        let cold = semantic_search(&db);
        let loads_after_cold = crate::vector_store::persisted_hnsw_load_count();
        let warm = semantic_search(&db);
        let loads_after_warm = crate::vector_store::persisted_hnsw_load_count();
        let lexical = search_with_mode(
            &db,
            "mixedcase token",
            1,
            SearchMode::Exact,
            RetrievalWeights::default(),
        )
        .expect("case insensitive FTS search");
        QueryEvidence {
            cold_ids: result_ids(&cold),
            warm_ids: result_ids(&warm),
            lexical_ids: result_ids(&lexical),
            lexical_mode: lexical.mode,
            checksum: checksum(&cold),
            loads_before,
            loads_after_cold,
            loads_after_warm,
            fts_ready: !lexical.results.is_empty(),
            manifest_generation: manifest.provider_generation,
            manifest_files,
            elapsed: started.elapsed(),
            query_norm: cold.query_embedding_norm.unwrap_or_default(),
        }
    }
}

fn semantic_search(db: &cozo::DbInstance) -> crate::retrieval::SearchResults {
    search_with_mode(
        db,
        "semantic concept",
        1,
        SearchMode::Semantic,
        RetrievalWeights::default(),
    )
    .expect("semantic search")
}

struct SourceEvidence {
    db_path: std::path::PathBuf,
    vector_count: usize,
    cache_count: usize,
    manifest_generation: Option<u64>,
    manifest_count: usize,
    indexed_count: usize,
    elapsed: Duration,
    canonical_handles_reused: bool,
}

impl SourceEvidence {
    fn assert_invariants(&self) {
        assert!(self.db_path.exists(), "source database must be persisted");
        assert_eq!(
            self.vector_count, 4,
            "prefix count must avoid vector query decoding"
        );
        assert_eq!(self.cache_count, 4, "embedding cache prefix count");
        assert_eq!(self.manifest_count, self.vector_count);
        assert_eq!(self.indexed_count, self.vector_count);
        assert!(self.manifest_generation.is_some());
        assert!(
            self.canonical_handles_reused,
            "canonical DB handles must be shared"
        );
    }

    fn print(&self) {
        println!(
            "EVIDENCE docs-source vectors={} cache={} manifest_generation={:?} manifest_count={} indexed={} canonical_reuse={} elapsed_ms={}",
            self.vector_count,
            self.cache_count,
            self.manifest_generation,
            self.manifest_count,
            self.indexed_count,
            self.canonical_handles_reused,
            self.elapsed.as_millis()
        );
    }
}

struct QueryEvidence {
    cold_ids: Vec<String>,
    warm_ids: Vec<String>,
    lexical_ids: Vec<String>,
    lexical_mode: SearchMode,
    checksum: String,
    loads_before: usize,
    loads_after_cold: usize,
    loads_after_warm: usize,
    fts_ready: bool,
    manifest_generation: Option<u64>,
    manifest_files: bool,
    elapsed: Duration,
    query_norm: f64,
}

impl QueryEvidence {
    fn assert_invariants(&self) {
        assert_eq!(self.cold_ids, ["chunk-semantic"]);
        assert_eq!(self.warm_ids, self.cold_ids);
        assert_eq!(self.lexical_ids, ["chunk-lexical"]);
        assert_eq!(self.lexical_mode, SearchMode::Exact);
        assert_eq!(
            self.loads_after_cold,
            self.loads_before + 1,
            "cold disk load required"
        );
        assert_eq!(
            self.loads_after_warm, self.loads_after_cold,
            "warm cache reused"
        );
        assert!(self.fts_ready, "FTS schema must backfill persisted rows");
        assert!(self.manifest_generation.is_some());
        assert!(
            self.manifest_files,
            "HNSW manifest and dump files must persist"
        );
        assert!(
            (self.query_norm - 1.0).abs() < 1e-6,
            "cosine query must be unit length"
        );
    }

    fn print(&self) {
        println!(
            "EVIDENCE docs-query direct_db=true cold={:?} warm={:?} lexical={:?} checksum={} hnsw_disk_loads={}/{}/{} hnsw_files={} fts=ready manifest_generation={:?} norm={:.3} elapsed_ms={}",
            self.cold_ids,
            self.warm_ids,
            self.lexical_ids,
            self.checksum,
            self.loads_before,
            self.loads_after_cold,
            self.loads_after_warm,
            self.manifest_files,
            self.manifest_generation,
            self.query_norm,
            self.elapsed.as_millis()
        );
    }
}

fn fixture_chunks() -> Vec<ChunkArtifact> {
    [
        ("chunk-lexical", "MixedCase TOKEN proves FTS backfill."),
        (
            "chunk-semantic",
            "Semantic target carries the concept signal.",
        ),
        ("chunk-other-a", "Unrelated archive record."),
        ("chunk-other-b", "Another unrelated corpus item."),
    ]
    .into_iter()
    .map(|(id, content)| ChunkArtifact {
        chunk_id: id.into(),
        document_id: format!("doc-{id}"),
        artifact_id: format!("artifact-{id}"),
        chunk_index: 0,
        page_start: 1,
        page_end: 1,
        content: content.into(),
        content_hash: format!("hash-{id}"),
        embedding_status: "pending".into(),
    })
    .collect()
}

fn canonical_handle_evidence(root: &std::path::Path) -> bool {
    let path = root.join("canonical.db");
    let first = crate::acquire_docs_db(&path).expect("acquire canonical database");
    let second = crate::acquire_docs_db(root.join(".").join("canonical.db"))
        .expect("acquire canonical alias");
    std::sync::Arc::ptr_eq(&first, &second)
}

fn hnsw_manifest_files(
    root: &std::path::Path,
    manifest: &crate::vector_store::HnswManifest,
) -> bool {
    let dir = root.join("vectors").join("hnsw").join(PROVIDER);
    dir.join("manifest.json").is_file()
        && dir
            .join(format!("{}.hnsw.graph", manifest.dump_basename))
            .is_file()
        && dir
            .join(format!("{}.hnsw.data", manifest.dump_basename))
            .is_file()
}

fn vector_for(value: &str) -> Vec<f32> {
    let lower = value.to_ascii_lowercase();
    if lower.contains("semantic") || lower.contains("concept") {
        vec![0.0, 1.0]
    } else if lower.contains("mixedcase") || lower.contains("token") {
        vec![1.0, 0.0]
    } else {
        vec![0.6, 0.8]
    }
}

fn result_ids(results: &crate::retrieval::SearchResults) -> Vec<String> {
    results
        .results
        .iter()
        .map(|result| result.chunk_id.clone())
        .collect()
}

fn checksum(results: &crate::retrieval::SearchResults) -> String {
    let joined = result_ids(results).join(",");
    blake3::hash(joined.as_bytes()).to_hex()[..12].to_string()
}

struct EnvGuard {
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &str, value: std::path::PathBuf) -> Self {
        let previous = std::env::var_os(key);
        // SAFETY: the serial test lock keeps this process-wide environment mutation isolated.
        unsafe { std::env::set_var(key, value) };
        Self { previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: this is the paired restoration under the serial test lock.
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var("ARCHON_DOC_VECTOR_STORE_DIR", value),
                None => std::env::remove_var("ARCHON_DOC_VECTOR_STORE_DIR"),
            }
        }
    }
}
