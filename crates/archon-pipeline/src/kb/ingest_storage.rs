//! Atomic, bounded CozoDB storage for knowledge-base ingest chunks.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use anyhow::Result;
use cozo::{DataValue, DbInstance, MultiTransaction};

use super::IngestResult;
use super::schema::KbNodeType;

pub(super) const KB_INGEST_BATCH_SIZE: usize = 64;

pub(super) struct ChunkData {
    pub(super) title: String,
    pub(super) content: String,
}

struct PendingChunk<'a> {
    chunk: &'a ChunkData,
    content_hash: String,
    chunk_index: usize,
}

/// Stores raw chunks and their keyed content hashes in the same transaction.
pub(super) struct ChunkStorage {
    db: DbInstance,
    fail_after_hash_write: AtomicBool,
    transaction_count: AtomicUsize,
}

impl ChunkStorage {
    pub(super) fn new(db: DbInstance) -> Self {
        Self {
            db,
            fail_after_hash_write: AtomicBool::new(false),
            transaction_count: AtomicUsize::new(0),
        }
    }

    pub(super) fn store(
        &self,
        chunks: &[ChunkData],
        source: &str,
        domain_tag: &str,
        hash: impl Fn(&str) -> String,
    ) -> Result<IngestResult> {
        let mut seen_hashes = self.load_existing_hashes()?;
        let mut pending = Vec::new();

        for (chunk_index, chunk) in chunks.iter().enumerate() {
            let content_hash = hash(&chunk.content);
            if seen_hashes.insert(content_hash.clone()) {
                pending.push(PendingChunk {
                    chunk,
                    content_hash,
                    chunk_index,
                });
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let mut nodes_created = 0;
        for batch in pending.chunks(KB_INGEST_BATCH_SIZE) {
            nodes_created += self.store_batch(batch, source, domain_tag, now)?;
        }

        Ok(IngestResult {
            nodes_created,
            chunks_processed: chunks.len(),
            errors: Vec::new(),
        })
    }

    fn load_existing_hashes(&self) -> Result<HashSet<String>> {
        let existing = self
            .db
            .run_script(
                "?[content_hash] := *kb_content_hashes{content_hash}",
                BTreeMap::new(),
                cozo::ScriptMutability::Immutable,
            )
            .map_err(|error| anyhow::anyhow!("load existing content hashes failed: {error}"))?;
        Ok(existing
            .rows
            .iter()
            .filter_map(|row| row[0].get_str().map(str::to_owned))
            .collect())
    }

    fn store_batch(
        &self,
        batch: &[PendingChunk<'_>],
        source: &str,
        domain_tag: &str,
        now: f64,
    ) -> Result<usize> {
        let transaction = self.db.multi_transaction(true);
        self.transaction_count.fetch_add(1, Ordering::Relaxed);
        let stored = self.store_batch_in_transaction(&transaction, batch, source, domain_tag, now);
        match stored {
            Ok(nodes_created) => transaction
                .commit()
                .map(|()| nodes_created)
                .map_err(|error| anyhow::anyhow!("commit KB ingest batch failed: {error}")),
            Err(error) => {
                let _ = transaction.abort();
                Err(error)
            }
        }
    }

    fn store_batch_in_transaction(
        &self,
        transaction: &MultiTransaction,
        batch: &[PendingChunk<'_>],
        source: &str,
        domain_tag: &str,
        now: f64,
    ) -> Result<usize> {
        let batch_hashes = DataValue::List(
            batch
                .iter()
                .map(|chunk| DataValue::from(chunk.content_hash.as_str()))
                .collect(),
        );
        let mut params = BTreeMap::new();
        params.insert("hashes".to_string(), batch_hashes);
        let existing = transaction
            .run_script(
                "?[content_hash] := *kb_content_hashes{content_hash}, content_hash in $hashes",
                params,
            )
            .map_err(|error| anyhow::anyhow!("check batch content hashes failed: {error}"))?;
        let existing: HashSet<String> = existing
            .rows
            .iter()
            .filter_map(|row| row[0].get_str().map(str::to_owned))
            .collect();
        let new_chunks: Vec<_> = batch
            .iter()
            .filter(|chunk| !existing.contains(&chunk.content_hash))
            .collect();
        if new_chunks.is_empty() {
            return Ok(0);
        }

        let rows: Vec<(String, &PendingChunk<'_>)> = new_chunks
            .iter()
            .map(|chunk| (uuid::Uuid::new_v4().to_string(), *chunk))
            .collect();
        self.insert_hash_rows(transaction, &rows)?;
        if self.fail_after_hash_write.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected KB ingest batch failure after hash write");
        }
        self.insert_node_rows(transaction, &rows, source, domain_tag, now)?;
        Ok(rows.len())
    }

    fn insert_hash_rows(
        &self,
        transaction: &MultiTransaction,
        rows: &[(String, &PendingChunk<'_>)],
    ) -> Result<()> {
        let hash_rows = DataValue::List(
            rows.iter()
                .map(|(node_id, chunk)| {
                    DataValue::List(vec![
                        DataValue::from(chunk.content_hash.as_str()),
                        DataValue::from(node_id.as_str()),
                    ])
                })
                .collect(),
        );
        let mut params = BTreeMap::new();
        params.insert("rows".to_string(), hash_rows);
        transaction
            .run_script(
                "?[content_hash, node_id] <- $rows\n                 :insert kb_content_hashes { content_hash => node_id }",
                params,
            )
            .map_err(|error| anyhow::anyhow!("reserve content hashes failed: {error}"))?;
        Ok(())
    }

    fn insert_node_rows(
        &self,
        transaction: &MultiTransaction,
        rows: &[(String, &PendingChunk<'_>)],
        source: &str,
        domain_tag: &str,
        now: f64,
    ) -> Result<()> {
        let node_rows = DataValue::List(
            rows.iter()
                .map(|(node_id, chunk)| {
                    DataValue::List(vec![
                        DataValue::from(node_id.as_str()),
                        DataValue::from(node_type_str(&KbNodeType::Raw)),
                        DataValue::from(source),
                        DataValue::from(domain_tag),
                        DataValue::from(chunk.chunk.title.as_str()),
                        DataValue::from(chunk.chunk.content.as_str()),
                        DataValue::from(chunk.content_hash.as_str()),
                        DataValue::from(chunk.chunk_index as i64),
                        DataValue::from(now),
                        DataValue::from(now),
                    ])
                })
                .collect(),
        );
        let mut params = BTreeMap::new();
        params.insert("rows".to_string(), node_rows);
        transaction
            .run_script(
                "?[node_id, node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at] <- $rows\n                 :put kb_nodes { node_id => node_type, source, domain_tag, title, content, content_hash, chunk_index, created_at, updated_at }",
                params,
            )
            .map_err(|error| anyhow::anyhow!("insert KB nodes failed: {error}"))?;
        Ok(())
    }

    #[doc(hidden)]
    pub fn fail_next_batch_after_hash_write_for_tests(&self) {
        self.fail_after_hash_write.store(true, Ordering::SeqCst);
    }

    #[doc(hidden)]
    pub fn transaction_count_for_tests(&self) -> usize {
        self.transaction_count.load(Ordering::Relaxed)
    }
}

fn node_type_str(node_type: &KbNodeType) -> &'static str {
    match node_type {
        KbNodeType::Raw => "raw",
        KbNodeType::Compiled => "compiled",
        KbNodeType::Concept => "concept",
        KbNodeType::Answer => "answer",
        KbNodeType::Index => "index",
    }
}
