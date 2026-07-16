use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use archon_memory::embedding::EmbeddingProvider;
use archon_memory::types::MemoryError;
use cozo::{DataValue, DbInstance, ScriptMutability};

use super::{EmbeddingConfig, EmbeddingProviderKind, Indexer};

fn test_db() -> DbInstance {
    DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB")
}

fn mock_indexer(db: DbInstance) -> Indexer {
    Indexer::new(
        db,
        EmbeddingConfig {
            provider: EmbeddingProviderKind::Mock,
            dimension: 8,
        },
        None,
    )
    .expect("indexer")
}

fn chunks_for_file(db: &DbInstance, file_path: &str) -> Vec<Vec<DataValue>> {
    let mut params = BTreeMap::new();
    params.insert("fp".to_string(), DataValue::from(file_path));
    db.run_script(
        "?[chunk_id, chunk_content, file_hash] := *code_chunks{chunk_id, file_path, chunk_content, file_hash}, file_path = $fp",
        params,
        ScriptMutability::Immutable,
    )
    .expect("chunk query")
    .rows
}

fn file_state_hash(db: &DbInstance, file_path: &str) -> Option<String> {
    let mut params = BTreeMap::new();
    params.insert("fp".to_string(), DataValue::from(file_path));
    db.run_script(
        "?[file_hash] := *file_states{file_path, file_hash}, file_path = $fp",
        params,
        ScriptMutability::Immutable,
    )
    .ok()?
    .rows
    .first()
    .and_then(|row| row.first())
    .and_then(|value| value.get_str())
    .map(str::to_owned)
}

fn write_rust(path: &Path, source: &str) {
    std::fs::write(path, source).expect("write source")
}

struct FailingEmbedder;

impl EmbeddingProvider for FailingEmbedder {
    fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Err(MemoryError::Embedding(
            "intentional test failure".to_string(),
        ))
    }

    fn dimensions(&self) -> usize {
        8
    }
}

struct CountingEmbedder {
    batches: Mutex<Vec<usize>>,
}

impl CountingEmbedder {
    fn batches(&self) -> Vec<usize> {
        self.batches.lock().unwrap().clone()
    }
}

impl EmbeddingProvider for CountingEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        self.batches.lock().unwrap().push(texts.len());
        Ok(texts.iter().map(|_| vec![0.0; 8]).collect())
    }

    fn dimensions(&self) -> usize {
        8
    }
}

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

struct MismatchedEmbedder;

impl EmbeddingProvider for MismatchedEmbedder {
    fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Ok(Vec::new())
    }

    fn dimensions(&self) -> usize {
        8
    }
}

struct WrongDimensionEmbedder;

impl EmbeddingProvider for WrongDimensionEmbedder {
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, MemoryError> {
        Ok(texts.iter().map(|_| vec![0.0; 7]).collect())
    }

    fn dimensions(&self) -> usize {
        8
    }
}

#[tokio::test]
async fn unchanged_file_uses_keyed_file_state() {
    let db = test_db();
    let indexer = mock_indexer(db.clone());
    indexer.ensure_schema().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("unchanged.rs");
    write_rust(&file, "fn original() {}\n");

    indexer.index_file(&file).await.unwrap();
    let file_path = file.to_string_lossy().to_string();
    let original_state = file_state_hash(&db, &file_path).expect("keyed file state");

    indexer.remove_file_chunks(&file_path).unwrap();
    indexer.index_file(&file).await.unwrap();

    assert!(chunks_for_file(&db, &file_path).is_empty());
    assert_eq!(
        file_state_hash(&db, &file_path).as_deref(),
        Some(original_state.as_str())
    );
}

#[tokio::test]
async fn replacement_writes_all_new_chunks_in_one_file_commit() {
    let db = test_db();
    let indexer = mock_indexer(db.clone());
    indexer.ensure_schema().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("replace.rs");
    write_rust(&file, "fn old() {}\n");
    indexer.index_file(&file).await.unwrap();

    write_rust(&file, "fn first() {}\nfn second() {}\n");
    indexer.index_file(&file).await.unwrap();

    let file_path = file.to_string_lossy().to_string();
    let chunks = chunks_for_file(&db, &file_path);
    assert_eq!(chunks.len(), 2);
    assert!(
        chunks
            .iter()
            .all(|row| row[1].get_str().unwrap().contains("fn"))
    );
    assert!(
        chunks
            .iter()
            .all(|row| !row[1].get_str().unwrap().contains("old"))
    );
    assert!(
        chunks
            .iter()
            .all(|row| Some(row[2].get_str().unwrap())
                == file_state_hash(&db, &file_path).as_deref())
    );
}

#[tokio::test]
async fn embedding_failure_preserves_old_chunks_and_file_state() {
    let db = test_db();
    let mut indexer = mock_indexer(db.clone());
    indexer.ensure_schema().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("failure.rs");
    write_rust(&file, "fn old() {}\n");
    indexer.index_file(&file).await.unwrap();

    let file_path = file.to_string_lossy().to_string();
    let old_chunks = chunks_for_file(&db, &file_path);
    let old_state = file_state_hash(&db, &file_path);
    write_rust(&file, "fn replacement() {}\n");
    indexer.embedder = Arc::new(FailingEmbedder);

    assert!(indexer.index_file(&file).await.is_err());
    assert_eq!(chunks_for_file(&db, &file_path), old_chunks);
    assert_eq!(file_state_hash(&db, &file_path), old_state);
}

#[tokio::test]
async fn zero_chunk_change_removes_chunks_and_records_state() {
    let db = test_db();
    let indexer = mock_indexer(db.clone());
    indexer.ensure_schema().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("empty.rs");
    write_rust(&file, "fn original() {}\n");
    indexer.index_file(&file).await.unwrap();

    write_rust(&file, "");
    indexer.index_file(&file).await.unwrap();

    let file_path = file.to_string_lossy().to_string();
    assert!(chunks_for_file(&db, &file_path).is_empty());
    assert!(file_state_hash(&db, &file_path).is_some());
    indexer.index_file(&file).await.unwrap();
    assert!(chunks_for_file(&db, &file_path).is_empty());
}

#[test]
fn first_repository_run_populates_file_states_without_losing_existing_chunks() {
    let db = test_db();
    let indexer = mock_indexer(db.clone());
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("legacy.rs");
    write_rust(&file, "fn legacy() {}\n");
    let file_path = file.to_string_lossy().to_string();

    indexer.ensure_schema().unwrap();
    indexer
        .index_changed_file(&file, "fn legacy() {}\n", "rust", None)
        .unwrap();
    let original_chunks = chunks_for_file(&db, &file_path);
    db.run_script(
        "?[file_path] := *file_states{file_path} :rm file_states { file_path }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    assert!(file_state_hash(&db, &file_path).is_none());

    indexer
        .index_repository_blocking(
            tmp.path(),
            &crate::metadata::IndexConfig {
                root_path: tmp.path().to_path_buf(),
                include_patterns: Vec::new(),
                exclude_patterns: Vec::new(),
            },
        )
        .unwrap();

    assert_eq!(
        chunks_for_file(&db, &file_path).len(),
        original_chunks.len()
    );
    assert!(file_state_hash(&db, &file_path).is_some());
}

#[test]
fn repository_embeddings_batch_chunks_across_files() {
    let db = test_db();
    let mut indexer = mock_indexer(db);
    indexer.ensure_schema().unwrap();
    let embedder = Arc::new(CountingEmbedder {
        batches: Mutex::new(Vec::new()),
    });
    indexer.embedder = embedder.clone();
    let tmp = tempfile::tempdir().unwrap();
    write_rust(&tmp.path().join("one.rs"), "fn one() {}\n");
    write_rust(&tmp.path().join("two.rs"), "fn two() {}\n");

    indexer
        .index_repository_blocking(
            tmp.path(),
            &crate::metadata::IndexConfig {
                root_path: tmp.path().to_path_buf(),
                include_patterns: Vec::new(),
                exclude_patterns: Vec::new(),
            },
        )
        .unwrap();

    assert_eq!(embedder.batches(), vec![2]);
}

#[tokio::test]
async fn cancellation_after_embedding_preserves_uncommitted_file() {
    let db = test_db();
    let mut indexer = mock_indexer(db.clone());
    indexer.ensure_schema().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("cancel.rs");
    write_rust(&file, "fn old() {}\n");
    indexer.index_file(&file).await.unwrap();

    let file_path = file.to_string_lossy().to_string();
    let old_chunks = chunks_for_file(&db, &file_path);
    let old_state = file_state_hash(&db, &file_path);
    write_rust(&file, "fn replacement() {}\n");
    let cancel = Arc::new(AtomicBool::new(false));
    indexer.embedder = Arc::new(CancellingEmbedder {
        cancel: cancel.clone(),
    });

    assert_eq!(
        indexer
            .index_changed_file(&file, "fn replacement() {}\n", "rust", Some(&cancel))
            .unwrap(),
        None
    );
    assert_eq!(chunks_for_file(&db, &file_path), old_chunks);
    assert_eq!(file_state_hash(&db, &file_path), old_state);
}

#[test]
fn cancellation_before_commit_rolls_back_replacement_mutations() {
    let db = test_db();
    let indexer = mock_indexer(db.clone());
    indexer.ensure_schema().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("commit-cancel.rs");
    write_rust(&file, "fn old() {}\n");
    indexer
        .index_changed_file(&file, "fn old() {}\n", "rust", None)
        .unwrap();

    let file_path = file.to_string_lossy().to_string();
    let old_chunks = chunks_for_file(&db, &file_path);
    let old_state = file_state_hash(&db, &file_path);
    let replacement = "fn replacement() {}\n";
    let chunks = indexer
        .chunker
        .chunk_file(&file, replacement, super::Language::Rust);
    let cancel = AtomicBool::new(false);
    let prepared = indexer
        .prepare_chunks(chunks, Some(&cancel))
        .unwrap()
        .unwrap();

    let outcome = indexer
        .file_store()
        .replace_file_with_cancel(
            &file_path,
            &super::sha256_hex(replacement),
            &prepared,
            || {
                cancel.store(true, Ordering::Relaxed);
                cancel.load(Ordering::Relaxed)
            },
        )
        .unwrap();

    assert!(matches!(
        outcome,
        super::super::index_storage::ReplaceFileOutcome::Cancelled
    ));
    assert_eq!(chunks_for_file(&db, &file_path), old_chunks);
    assert_eq!(file_state_hash(&db, &file_path), old_state);
}

#[test]
fn repository_empty_recognized_file_records_state_without_counting_file() {
    let db = test_db();
    let indexer = mock_indexer(db.clone());
    indexer.ensure_schema().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("empty.rs");
    write_rust(&file, "");

    let stats = indexer
        .index_repository_blocking(
            tmp.path(),
            &crate::metadata::IndexConfig {
                root_path: tmp.path().to_path_buf(),
                include_patterns: Vec::new(),
                exclude_patterns: Vec::new(),
            },
        )
        .unwrap();

    assert_eq!(stats.total_files, 0);
    assert_eq!(stats.total_chunks, 0);
    assert!(stats.languages.is_empty());
    assert!(file_state_hash(&db, &file.to_string_lossy()).is_some());
}

#[tokio::test]
async fn embedding_count_mismatch_preserves_old_chunks_and_file_state() {
    let db = test_db();
    let mut indexer = mock_indexer(db.clone());
    indexer.ensure_schema().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("mismatch.rs");
    write_rust(&file, "fn old() {}\n");
    indexer.index_file(&file).await.unwrap();

    let file_path = file.to_string_lossy().to_string();
    let old_chunks = chunks_for_file(&db, &file_path);
    let old_state = file_state_hash(&db, &file_path);
    write_rust(&file, "fn replacement() {}\n");
    indexer.embedder = Arc::new(MismatchedEmbedder);

    assert!(indexer.index_file(&file).await.is_err());
    assert_eq!(chunks_for_file(&db, &file_path), old_chunks);
    assert_eq!(file_state_hash(&db, &file_path), old_state);
}

#[tokio::test]
async fn transaction_failure_after_chunk_deletion_rolls_back_old_file() {
    let db = test_db();
    let mut indexer = mock_indexer(db.clone());
    indexer.ensure_schema().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("rollback.rs");
    write_rust(&file, "fn old() {}\n");
    indexer.index_file(&file).await.unwrap();

    let file_path = file.to_string_lossy().to_string();
    let old_chunks = chunks_for_file(&db, &file_path);
    let old_state = file_state_hash(&db, &file_path);
    write_rust(&file, "fn replacement() {}\n");
    indexer.embedder = Arc::new(WrongDimensionEmbedder);

    assert!(indexer.index_file(&file).await.is_err());
    assert_eq!(chunks_for_file(&db, &file_path), old_chunks);
    assert_eq!(file_state_hash(&db, &file_path), old_state);
}
