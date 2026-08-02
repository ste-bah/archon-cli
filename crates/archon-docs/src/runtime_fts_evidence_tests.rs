use std::collections::BTreeMap;
use std::ffi::OsString;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::retrieval::{RetrievalWeights, SearchMode, search_with_mode};

#[test]
#[serial_test::serial(docs_global_state)]
fn persisted_pre_fts_migration_backfills_legacy_rows() {
    let evidence = LegacyFixture::new().migrate_legacy_fts();
    evidence.assert_invariants();
    evidence.print();
}

struct LegacyFixture {
    temp: tempfile::TempDir,
    _vector_env: EnvGuard,
}

impl LegacyFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("create persisted evidence fixture");
        let vector_env = EnvGuard::set("ARCHON_DOC_VECTOR_STORE_DIR", temp.path().join("vectors"));
        Self {
            temp,
            _vector_env: vector_env,
        }
    }

    fn migrate_legacy_fts(self) -> MigrationEvidence {
        let db_path = self.temp.path().join("legacy.sqlite");
        create_legacy_docs_db(&db_path);
        let precondition = legacy_rows(&db_path);
        let db =
            crate::acquire_docs_db(&db_path).expect("open legacy database through production path");
        let stemmed_index_ids = fts_chunk_ids(&db, "chunk_content_fts", "migrating");
        let exact_index_ids = fts_chunk_ids(&db, "chunk_exact_fts", "mixedcase token");
        let exact_mode = search_with_mode(
            &db,
            "mixedcase token",
            1,
            SearchMode::Exact,
            RetrievalWeights::default(),
        )
        .expect("search production exact mode after FTS backfill");
        MigrationEvidence {
            precondition,
            stemmed_index_ids,
            exact_index_ids,
            exact_mode: exact_mode.mode,
        }
    }
}

struct MigrationEvidence {
    precondition: LegacyPrecondition,
    stemmed_index_ids: Vec<String>,
    exact_index_ids: Vec<String>,
    exact_mode: SearchMode,
}

impl MigrationEvidence {
    fn assert_invariants(&self) {
        assert_eq!(self.precondition.ids, ["legacy-mixed", "legacy-stemmed"]);
        assert!(
            self.precondition.db_bytes > 0,
            "legacy database must be persisted"
        );
        assert_eq!(self.stemmed_index_ids, ["legacy-stemmed"]);
        assert_eq!(self.exact_index_ids, ["legacy-mixed"]);
        assert_eq!(self.exact_mode, SearchMode::Exact);
    }

    fn print(&self) {
        println!(
            "EVIDENCE docs-legacy-fts pre_rows={:?} db_bytes={} chunk_content_fts={:?} chunk_exact_fts={:?} production_exact_mode={} fts=backfilled",
            self.precondition.ids,
            self.precondition.db_bytes,
            self.stemmed_index_ids,
            self.exact_index_ids,
            self.exact_mode.as_str(),
        );
    }
}

struct LegacyPrecondition {
    ids: Vec<String>,
    db_bytes: u64,
}

fn create_legacy_docs_db(path: &std::path::Path) {
    let db = cozo::DbInstance::new("sqlite", path, "")
        .expect("create direct legacy SQLite Cozo database");
    db.run_script(
        r#":create doc_chunks {
            chunk_id: String =>
            document_id: String,
            artifact_id: String,
            chunk_index: Int,
            page_start: Int,
            page_end: Int,
            content: String,
            content_hash: String,
            embedding_status: String default "pending",
        }"#,
        Default::default(),
        cozo::ScriptMutability::Mutable,
    )
    .expect("create legacy doc_chunks only");
    for (id, content) in [
        ("legacy-mixed", "MixedCase TOKEN exact phrase lives here."),
        ("legacy-stemmed", "The system migrates durable records."),
    ] {
        db.run_script(
            "?[chunk_id, document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status] \
             <- [[$chunk_id, $document_id, $artifact_id, 0, 1, 1, $content, $content_hash, \"pending\"]] \
             :put doc_chunks { chunk_id => document_id, artifact_id, chunk_index, page_start, page_end, content, content_hash, embedding_status }",
            legacy_chunk_params(id, content),
            cozo::ScriptMutability::Mutable,
        )
        .expect("insert legacy doc chunk");
    }
}

fn legacy_chunk_params(
    id: &str,
    content: &str,
) -> std::collections::BTreeMap<String, cozo::DataValue> {
    [
        ("chunk_id", id),
        ("document_id", "legacy-doc"),
        ("artifact_id", "legacy-artifact"),
        ("content", content),
        ("content_hash", id),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), cozo::DataValue::from(value)))
    .collect()
}

fn legacy_rows(path: &std::path::Path) -> LegacyPrecondition {
    let db = cozo::DbInstance::new("sqlite", path, "").expect("reopen direct legacy database");
    let rows = db
        .run_script(
            "?[chunk_id] := *doc_chunks{chunk_id} :order chunk_id",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .expect("read physical legacy rows");
    LegacyPrecondition {
        ids: rows
            .rows
            .iter()
            .map(|row| row[0].get_str().unwrap_or("").to_string())
            .collect(),
        db_bytes: std::fs::metadata(path).expect("stat legacy database").len(),
    }
}

fn fts_chunk_ids(db: &DbInstance, index: &str, query: &str) -> Vec<String> {
    let script = match index {
        "chunk_content_fts" => FTS_CONTENT_IDS_SCRIPT,
        "chunk_exact_fts" => FTS_EXACT_IDS_SCRIPT,
        _ => panic!("unsupported physical FTS index: {index}"),
    };
    let params = [
        ("query".to_string(), DataValue::from(query)),
        ("k".to_string(), DataValue::from(1_i64)),
    ]
    .into_iter()
    .collect::<BTreeMap<_, _>>();
    db.run_script(script, params, ScriptMutability::Immutable)
        .expect("query physical FTS index")
        .rows
        .iter()
        .map(|row| row[1].get_str().unwrap_or("").to_string())
        .collect()
}

const FTS_CONTENT_IDS_SCRIPT: &str = "?[score, chunk_id] := \
    ~doc_chunks:chunk_content_fts {chunk_id | \
        query: $query, k: $k, score_kind: 'tf_idf', bind_score: score \
    } :order -score";
const FTS_EXACT_IDS_SCRIPT: &str = "?[score, chunk_id] := \
    ~doc_chunks:chunk_exact_fts {chunk_id | \
        query: $query, k: $k, score_kind: 'tf_idf', bind_score: score \
    } :order -score";

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
