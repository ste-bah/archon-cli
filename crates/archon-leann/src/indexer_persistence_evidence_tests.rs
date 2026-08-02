use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use archon_memory::embedding::EmbeddingProvider;
use archon_memory::types::MemoryError;
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::{EmbeddingConfig, EmbeddingProviderKind, Indexer};

const FILES: [&str; 3] = ["first.rs", "second.rs", "third.rs"];

type Baseline = Vec<(String, Vec<Vec<DataValue>>, Option<String>)>;

struct CancellingEmbedder {
    cancel: Arc<AtomicBool>,
}

impl EmbeddingProvider for CancellingEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        self.cancel.store(true, Ordering::Relaxed);
        Ok(texts.iter().map(|_| vec![0.0; 8]).collect())
    }

    fn dimensions(&self) -> usize {
        8
    }
}

#[test]
fn persisted_repository_cancellation_preserves_complete_file_rows_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let database_path = temp.path().join("leann.db");
    let database = open_database(&database_path);
    let mut indexer = mock_indexer(database.clone());
    indexer.ensure_schema().unwrap();
    let config = index_config(temp.path());
    write_sources(temp.path(), "baseline");
    indexer
        .index_repository_blocking(temp.path(), &config)
        .unwrap();
    let baseline = capture_baseline(&database, temp.path());
    write_sources(temp.path(), "replacement");
    let cancel = Arc::new(AtomicBool::new(false));
    indexer.embedder = Arc::new(CancellingEmbedder {
        cancel: Arc::clone(&cancel),
    });
    let stats = indexer
        .index_repository_blocking_with_cancel(temp.path(), &config, &cancel)
        .unwrap();
    assert_eq!(stats.total_files, 0);
    assert!(cancel.load(Ordering::Relaxed));
    drop(indexer);
    drop(database);
    verify_reopened(&database_path, &baseline);
}

fn open_database(path: &std::path::Path) -> DbInstance {
    DbInstance::new("sqlite", path.to_str().unwrap(), "").unwrap()
}

fn mock_indexer(database: DbInstance) -> Indexer {
    Indexer::new(
        database,
        EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        },
        None,
    )
    .unwrap()
}

fn index_config(root: &std::path::Path) -> crate::metadata::IndexConfig {
    crate::metadata::IndexConfig {
        root_path: root.to_path_buf(),
        include_patterns: Vec::new(),
        exclude_patterns: Vec::new(),
    }
}

fn write_sources(root: &std::path::Path, version: &str) {
    for file in FILES {
        std::fs::write(root.join(file), format!("fn {version}_{file}() {{}}\n")).unwrap();
    }
}

fn capture_baseline(database: &DbInstance, root: &std::path::Path) -> Baseline {
    FILES
        .iter()
        .map(|file| capture_file(database, &root.join(file).to_string_lossy()))
        .collect()
}

fn capture_file(
    database: &DbInstance,
    file_path: &str,
) -> (String, Vec<Vec<DataValue>>, Option<String>) {
    (
        file_path.into(),
        chunks_for_file(database, file_path),
        file_state_hash(database, file_path),
    )
}

fn verify_reopened(database_path: &std::path::Path, baseline: &Baseline) {
    let reopened = open_database(database_path);
    mock_indexer(reopened.clone()).ensure_schema().unwrap();
    for (path, chunks, state) in baseline {
        assert_eq!(chunks_for_file(&reopened, path), *chunks);
        assert_eq!(file_state_hash(&reopened, path), *state);
    }
    let chunk_count = total_chunks(baseline);
    assert_eq!(query_chunk_count(&reopened), chunk_count);
    println!(
        "EVIDENCE leann_persisted_interruption cancellation_phase=embedding_before_replacement_commits preserved_files=3 atomicity=per_file committed_chunks={chunk_count} reopened_chunks={}",
        query_chunk_count(&reopened),
    );
}

fn chunks_for_file(database: &DbInstance, file_path: &str) -> Vec<Vec<DataValue>> {
    database
        .run_script(
            "?[chunk_id, chunk_content, file_hash] := *code_chunks{chunk_id, file_path, chunk_content, file_hash}, file_path = $path",
            BTreeMap::from([("path".into(), DataValue::from(file_path))]),
            ScriptMutability::Immutable,
        )
        .unwrap()
        .rows
}

fn file_state_hash(database: &DbInstance, file_path: &str) -> Option<String> {
    database
        .run_script(
            "?[file_hash] := *file_states{file_path, file_hash}, file_path = $path",
            BTreeMap::from([("path".into(), DataValue::from(file_path))]),
            ScriptMutability::Immutable,
        )
        .ok()?
        .rows
        .first()?
        .first()?
        .get_str()
        .map(str::to_owned)
}

fn total_chunks(baseline: &Baseline) -> usize {
    baseline.iter().map(|(_, chunks, _)| chunks.len()).sum()
}

fn query_chunk_count(database: &DbInstance) -> usize {
    database
        .run_script(
            "?[count(chunk_id)] := *code_chunks{chunk_id}",
            BTreeMap::new(),
            ScriptMutability::Immutable,
        )
        .unwrap()
        .rows[0][0]
        .get_int()
        .unwrap() as usize
}
