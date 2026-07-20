use std::sync::{Mutex, mpsc};
use std::time::Duration;

use archon_pipeline::kb::schema::ensure_kb_schema;
use archon_pipeline::kb::{IngestSource, KnowledgeBase, QueryOptions};
use cozo::{DbInstance, ScriptMutability};

struct SynonymEmbeddingProvider;

impl archon_docs::embed::LocalEmbeddingProvider for SynonymEmbeddingProvider {
    fn embed_chunks(
        &self,
        chunks: &[String],
    ) -> Result<Vec<Vec<f32>>, archon_docs::errors::DocsError> {
        Ok(chunks.iter().map(|chunk| semantic_vector(chunk)).collect())
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, archon_docs::errors::DocsError> {
        Ok(semantic_vector(query))
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        "synonym-test"
    }
}

struct FailingMigrationProvider;

impl archon_docs::embed::LocalEmbeddingProvider for FailingMigrationProvider {
    fn embed_chunks(
        &self,
        _chunks: &[String],
    ) -> Result<Vec<Vec<f32>>, archon_docs::errors::DocsError> {
        Err(archon_docs::errors::DocsError::Embedding {
            message: "injected migration failure".to_string(),
        })
    }

    fn embed_query(&self, _query: &str) -> Result<Vec<f32>, archon_docs::errors::DocsError> {
        unreachable!()
    }

    fn dimension(&self) -> usize {
        3
    }

    fn backend_name(&self) -> &'static str {
        "failing-migration-test"
    }
}

struct VersionedEmbeddingProvider {
    model: &'static str,
}

impl archon_docs::embed::LocalEmbeddingProvider for VersionedEmbeddingProvider {
    fn embed_chunks(
        &self,
        chunks: &[String],
    ) -> Result<Vec<Vec<f32>>, archon_docs::errors::DocsError> {
        Ok(chunks
            .iter()
            .map(|chunk| versioned_vector(self.model, chunk))
            .collect())
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, archon_docs::errors::DocsError> {
        Ok(versioned_vector(self.model, query))
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        "versioned-test"
    }

    fn embedding_space_id(&self) -> String {
        format!("versioned-test:{}", self.model)
    }
}

struct PausedMigrationProvider {
    started: mpsc::SyncSender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
}

impl archon_docs::embed::LocalEmbeddingProvider for PausedMigrationProvider {
    fn embed_chunks(
        &self,
        chunks: &[String],
    ) -> Result<Vec<Vec<f32>>, archon_docs::errors::DocsError> {
        self.started.send(()).unwrap();
        self.resume.lock().unwrap().recv().unwrap();
        Ok(chunks.iter().map(|chunk| semantic_vector(chunk)).collect())
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, archon_docs::errors::DocsError> {
        Ok(semantic_vector(query))
    }

    fn dimension(&self) -> usize {
        2
    }

    fn backend_name(&self) -> &'static str {
        "paused-migration-test"
    }
}

fn versioned_vector(model: &str, text: &str) -> Vec<f32> {
    let vector = semantic_vector(text);
    if model == "model-a" {
        vector
    } else {
        vec![vector[1], vector[0]]
    }
}

fn semantic_vector(text: &str) -> Vec<f32> {
    let text = text.to_ascii_lowercase();
    if text.contains("automobile") || text.contains("vehicle") {
        vec![1.0, 0.0]
    } else {
        vec![0.0, 1.0]
    }
}

fn mem_db() -> DbInstance {
    DbInstance::new("mem", "", "").unwrap()
}

#[tokio::test]
async fn semantic_query_retrieves_ingested_synonym_without_text_overlap() {
    let db = mem_db();
    let kb = KnowledgeBase::with_embedder(db, Box::new(SynonymEmbeddingProvider)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let vehicle_path = dir.path().join("vehicle.txt");
    let unrelated_path = dir.path().join("unrelated.txt");
    std::fs::write(&vehicle_path, "An automobile carries passengers on roads.").unwrap();
    std::fs::write(&unrelated_path, "A recipe combines flour and water.").unwrap();
    kb.ingest(&IngestSource::FilePath(vehicle_path))
        .await
        .unwrap();
    kb.ingest(&IngestSource::FilePath(unrelated_path))
        .await
        .unwrap();

    let result = kb.query("vehicle", &QueryOptions::default()).await.unwrap();

    assert!(!result.sources.is_empty());
    assert!(result.sources[0].content.contains("automobile"));
}

#[tokio::test]
async fn semantic_constructor_rebuilds_legacy_embedding_schema_and_indexes_existing_nodes() {
    let db = mem_db();
    ensure_kb_schema(&db).unwrap();
    db.run_script(
        ":create kb_embeddings { node_id: String => embedding: [Float] }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    db.run_script(
        r#"
        ?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] <- [
            ["legacy-node", "raw", "legacy", "", "Road transport", "An automobile carries passengers.", "legacy-hash", 0, 1.0, 1.0]
        ]
        :put kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }
        "#,
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    let kb = KnowledgeBase::with_embedder(db, Box::new(SynonymEmbeddingProvider)).unwrap();
    let result = kb.query("vehicle", &QueryOptions::default()).await.unwrap();

    assert!(!result.sources.is_empty());
    assert_eq!(result.sources[0].node_id, "legacy-node");
}

#[tokio::test]
async fn failed_provider_migration_preserves_previous_semantic_index() {
    let db = mem_db();
    ensure_kb_schema(&db).unwrap();
    db.run_script(
        r#"
        ?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] <- [
            ["legacy-node", "raw", "legacy", "", "Road transport", "An automobile carries passengers.", "legacy-hash", 0, 1.0, 1.0]
        ]
        :put kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }
        "#,
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    let previous =
        KnowledgeBase::with_embedder(db.clone(), Box::new(SynonymEmbeddingProvider)).unwrap();
    assert_eq!(
        previous
            .query("vehicle", &QueryOptions::default())
            .await
            .unwrap()
            .sources[0]
            .node_id,
        "legacy-node"
    );

    assert!(KnowledgeBase::with_embedder(db.clone(), Box::new(FailingMigrationProvider)).is_err());

    let restored = KnowledgeBase::with_embedder(db, Box::new(SynonymEmbeddingProvider)).unwrap();
    assert_eq!(
        restored
            .query("vehicle", &QueryOptions::default())
            .await
            .unwrap()
            .sources[0]
            .node_id,
        "legacy-node"
    );
}

#[tokio::test]
async fn changing_model_with_same_backend_and_dimension_rebuilds_embeddings() {
    let db = mem_db();
    ensure_kb_schema(&db).unwrap();
    db.run_script(
        r#"
        ?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] <- [
            ["vehicle", "raw", "legacy", "", "Road transport", "An automobile carries passengers.", "vehicle-hash", 0, 1.0, 1.0],
            ["recipe", "raw", "legacy", "", "Bread", "A recipe combines flour and water.", "recipe-hash", 0, 1.0, 1.0]
        ]
        :put kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }
        "#,
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();
    KnowledgeBase::with_embedder(
        db.clone(),
        Box::new(VersionedEmbeddingProvider { model: "model-a" }),
    )
    .unwrap();

    let rebuilt = KnowledgeBase::with_embedder(
        db,
        Box::new(VersionedEmbeddingProvider { model: "model-b" }),
    )
    .unwrap();
    let result = rebuilt
        .query("vehicle", &QueryOptions::default())
        .await
        .unwrap();

    assert_eq!(result.sources[0].node_id, "vehicle");
}

#[tokio::test]
async fn stale_provider_handle_cannot_write_after_embedding_space_migration() {
    let db = mem_db();
    let stale = KnowledgeBase::with_embedder(
        db.clone(),
        Box::new(VersionedEmbeddingProvider { model: "model-a" }),
    )
    .unwrap();
    KnowledgeBase::with_embedder(
        db,
        Box::new(VersionedEmbeddingProvider { model: "model-b" }),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stale.txt");
    std::fs::write(&path, "An automobile carries passengers.").unwrap();

    let error = stale
        .ingest(&IngestSource::FilePath(path))
        .await
        .unwrap_err();

    assert!(error.to_string().contains("embedding space changed"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_during_migration_does_not_leave_orphan_embedding() {
    let db = mem_db();
    let current = KnowledgeBase::with_embedder(
        db.clone(),
        Box::new(VersionedEmbeddingProvider { model: "model-a" }),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vehicle.txt");
    std::fs::write(&path, "An automobile carries passengers.").unwrap();
    current.ingest(&IngestSource::FilePath(path)).await.unwrap();
    let node_id = current.list().await.unwrap()[0].node_id.clone();
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (resume_tx, resume_rx) = mpsc::sync_channel(1);
    let migration_db = db.clone();
    let migration = std::thread::spawn(move || {
        KnowledgeBase::with_embedder(
            migration_db,
            Box::new(PausedMigrationProvider {
                started: started_tx,
                resume: Mutex::new(resume_rx),
            }),
        )
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let delete = tokio::spawn(async move { current.delete(&node_id).await });

    tokio::time::sleep(Duration::from_millis(50)).await;
    resume_tx.send(()).unwrap();
    migration.join().unwrap().unwrap();
    delete.await.unwrap().unwrap();

    let embeddings = db
        .run_script(
            "?[node_id] := *kb_embeddings{node_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert!(embeddings.rows.is_empty());
}

#[tokio::test]
async fn deleting_semantic_node_through_text_only_handle_removes_embedding() {
    let db = mem_db();
    let semantic =
        KnowledgeBase::with_embedder(db.clone(), Box::new(SynonymEmbeddingProvider)).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vehicle.txt");
    std::fs::write(&path, "An automobile carries passengers.").unwrap();
    semantic
        .ingest(&IngestSource::FilePath(path))
        .await
        .unwrap();
    let node_id = semantic.list().await.unwrap()[0].node_id.clone();
    let text_only = KnowledgeBase::new(db.clone()).unwrap();

    text_only.delete(&node_id).await.unwrap();

    let embeddings = db
        .run_script(
            "?[node_id] := *kb_embeddings{node_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    assert!(embeddings.rows.is_empty());
}
