//! Atomic, bounded CozoDB storage for knowledge-base ingest chunks.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(test)]
use super::ingest_storage_test_hooks::ReservationTestHooks;

use anyhow::Result;
use cozo::{DataValue, DbInstance, MultiTransaction, Vector};
use ndarray::Array1;

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
    embedding: Option<&'a [f32]>,
}

const RESERVATION_LOCK_CONTEXT: &str = "KB ingest content-hash reservation";

/// Stores raw chunks and their keyed content hashes in the same transaction.
pub(super) struct ChunkStorage {
    db: DbInstance,
    /// Sidecar write-lock file for the backing database, when it is persisted.
    ///
    /// `None` for in-memory stores, which cannot be opened twice, and for
    /// callers that never told us where the database lives.
    reservation_lock: Option<PathBuf>,
    fail_after_hash_write: AtomicBool,
    transaction_count: AtomicUsize,
    #[cfg(test)]
    pub(super) test_hooks: ReservationTestHooks,
}

impl ChunkStorage {
    /// Storage with no cross-handle serialisation.
    ///
    /// Correct only when nothing else can write this database: an in-memory
    /// store, or a single `DbInstance` shared by the whole process. Deliberately
    /// does *not* try to recover the path from the guard registry — that
    /// registry is keyed on the address of a `DbInstance`, and `DbInstance` is
    /// `Clone`, so a lookup here would miss for every clone and quietly leave
    /// reservations unserialised. Callers that know the path say so, via
    /// [`ChunkStorage::for_db_path`].
    pub(super) fn new(db: DbInstance) -> Self {
        Self::with_reservation_lock(db, None)
    }

    /// Build a storage that serialises reservations on `db_path`'s write lock.
    ///
    /// Required for correctness whenever more than one `DbInstance` — in this
    /// process or another — can be open on the same file. The in-transaction
    /// `:insert` conflict only compares against the transaction's own snapshot,
    /// so two handles that read before either commits both see a hash as absent
    /// and both create a node for it.
    pub(super) fn for_db_path(db: DbInstance, db_path: impl AsRef<Path>) -> Self {
        let lock = archon_cozo::write_lock_path_for_db(db_path);
        Self::with_reservation_lock(db, Some(lock))
    }

    fn with_reservation_lock(db: DbInstance, reservation_lock: Option<PathBuf>) -> Self {
        Self {
            db,
            reservation_lock,
            fail_after_hash_write: AtomicBool::new(false),
            transaction_count: AtomicUsize::new(0),
            #[cfg(test)]
            test_hooks: ReservationTestHooks::default(),
        }
    }

    pub(super) fn db(&self) -> &DbInstance {
        &self.db
    }

    /// Run `reserve` with exclusive access to the backing database.
    ///
    /// The blocking variant, not the fail-fast one: losing this race is not
    /// recoverable by retrying, because the whole point is that the read and
    /// the reservation must not be interleaved. It is re-entrant, so an ingest
    /// nested inside an already-guarded mutable operation on the same database
    /// runs inline rather than blocking on the lock its own thread holds.
    fn with_reservation_lock_held<T>(&self, reserve: impl FnOnce() -> Result<T>) -> Result<T> {
        match &self.reservation_lock {
            Some(path) => {
                archon_cozo::with_write_lock_blocking(path, RESERVATION_LOCK_CONTEXT, reserve)
            }
            None => reserve(),
        }
    }

    pub(super) fn store(
        &self,
        chunks: &[ChunkData],
        embeddings: Option<&[Vec<f32>]>,
        source: &str,
        domain_tag: &str,
        hash: impl Fn(&str) -> String,
    ) -> Result<IngestResult> {
        if let Some(embeddings) = embeddings
            && embeddings.len() != chunks.len()
        {
            anyhow::bail!(
                "KB embedder returned {} vectors for {} chunks",
                embeddings.len(),
                chunks.len()
            );
        }
        let mut seen_hashes = self.load_existing_hashes()?;
        let mut pending = Vec::new();

        for (chunk_index, chunk) in chunks.iter().enumerate() {
            let content_hash = hash(&chunk.content);
            if seen_hashes.insert(content_hash.clone()) {
                pending.push(PendingChunk {
                    chunk,
                    content_hash,
                    chunk_index,
                    embedding: embeddings.map(|values| values[chunk_index].as_slice()),
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
        match self.store_batch_attempt(batch, source, domain_tag, now) {
            Ok(nodes_created) => Ok(nodes_created),
            Err(error) if is_content_hash_reservation_conflict(&error) => {
                self.persist_conflict_after_abort_for_tests(batch);
                self.retry_verified_reservation_race(batch, source, domain_tag, now, error)
            }
            Err(error) if is_retryable_content_hash_batch_lock(&error) => {
                self.retry_retryable_reservation_lock(batch, source, domain_tag, now, error)
            }
            Err(error) => Err(error),
        }
    }

    fn store_batch_attempt(
        &self,
        batch: &[PendingChunk<'_>],
        source: &str,
        domain_tag: &str,
        now: f64,
    ) -> Result<usize> {
        self.with_reservation_lock_held(|| self.reserve_and_commit(batch, source, domain_tag, now))
    }

    /// The reserve-through-commit span: open a transaction, re-read the hashes
    /// for this batch, reserve the missing ones, and commit.
    ///
    /// Must run under [`Self::with_reservation_lock_held`]. The transaction is
    /// opened *inside* the lock so its snapshot already contains whatever the
    /// previous holder committed — reading before the lock is what let both
    /// writers conclude a shared hash was unclaimed.
    fn reserve_and_commit(
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

    #[cfg(test)]
    fn persist_conflict_after_abort_for_tests(&self, batch: &[PendingChunk<'_>]) {
        self.test_hooks
            .persist_conflict_after_abort(&batch[0].content_hash);
    }

    #[cfg(not(test))]
    fn persist_conflict_after_abort_for_tests(&self, _batch: &[PendingChunk<'_>]) {}

    fn retry_retryable_reservation_lock(
        &self,
        batch: &[PendingChunk<'_>],
        source: &str,
        domain_tag: &str,
        now: f64,
        mut error: anyhow::Error,
    ) -> Result<usize> {
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(5));
            match self.store_batch_attempt(batch, source, domain_tag, now) {
                Ok(nodes_created) => return Ok(nodes_created),
                Err(next_error) if is_content_hash_reservation_conflict(&next_error) => {
                    return self.retry_verified_reservation_race(
                        batch, source, domain_tag, now, next_error,
                    );
                }
                Err(next_error) if is_retryable_content_hash_batch_lock(&next_error) => {
                    error = next_error;
                }
                Err(next_error) => return Err(next_error),
            }
        }
        Err(error)
    }

    fn retry_verified_reservation_race(
        &self,
        batch: &[PendingChunk<'_>],
        source: &str,
        domain_tag: &str,
        now: f64,
        error: anyhow::Error,
    ) -> Result<usize> {
        if !is_content_hash_reservation_conflict(&error) {
            return Err(error);
        }
        for _ in 0..20 {
            match self.load_existing_hashes() {
                Ok(existing) => {
                    let remaining = pending_missing_chunks(batch, &existing);
                    if remaining.len() < batch.len() {
                        return if remaining.is_empty() {
                            Ok(0)
                        } else {
                            self.store_batch(&remaining, source, domain_tag, now)
                        };
                    }
                }
                Err(read_error) => return Err(read_error),
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Err(error)
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
        // Stays inside the reservation window: the defect this guards against
        // is two writers holding stale reads at this exact point. The hook is a
        // *bounded* rendezvous precisely so it survives being serialised — see
        // `ReservationRendezvous`.
        self.wait_before_hash_reservation();
        self.insert_hash_rows(transaction, &rows)?;
        if self.fail_after_hash_write.swap(false, Ordering::SeqCst) {
            anyhow::bail!("injected KB ingest batch failure after hash write");
        }
        self.insert_node_rows(transaction, &rows, source, domain_tag, now)?;
        self.insert_embedding_rows(transaction, &rows)?;
        Ok(rows.len())
    }

    fn insert_hash_rows(
        &self,
        transaction: &MultiTransaction,
        rows: &[(String, &PendingChunk<'_>)],
    ) -> Result<()> {
        self.inject_hash_reservation_failure_for_tests()?;
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
            .map_err(|error| {
                let details = error.chain().map(ToString::to_string).collect::<Vec<_>>().join(" :: ");
                anyhow::anyhow!("reserve content hashes failed: {details}")
            })?;
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

    fn insert_embedding_rows(
        &self,
        transaction: &MultiTransaction,
        rows: &[(String, &PendingChunk<'_>)],
    ) -> Result<()> {
        let embedding_rows: Vec<_> = rows
            .iter()
            .filter_map(|(node_id, chunk)| {
                chunk.embedding.map(|embedding| {
                    DataValue::List(vec![
                        DataValue::from(node_id.as_str()),
                        DataValue::Vec(Vector::F32(Array1::from_vec(embedding.to_vec()))),
                    ])
                })
            })
            .collect();
        if embedding_rows.is_empty() {
            return Ok(());
        }
        let mut params = BTreeMap::new();
        params.insert("rows".to_string(), DataValue::List(embedding_rows));
        transaction
            .run_script(
                "?[node_id, embedding] <- $rows\n                 :put kb_embeddings { node_id => embedding }",
                params,
            )
            .map_err(|error| anyhow::anyhow!("insert KB embeddings failed: {error}"))?;
        Ok(())
    }

    #[cfg(test)]
    fn inject_hash_reservation_failure_for_tests(&self) -> Result<()> {
        self.test_hooks.inject_failure()
    }

    #[cfg(not(test))]
    fn inject_hash_reservation_failure_for_tests(&self) -> Result<()> {
        Ok(())
    }

    #[cfg(test)]
    fn wait_before_hash_reservation(&self) {
        self.test_hooks.wait_before_reservation();
    }

    #[cfg(not(test))]
    fn wait_before_hash_reservation(&self) {}

    #[doc(hidden)]
    pub fn fail_next_batch_after_hash_write_for_tests(&self) {
        self.fail_after_hash_write.store(true, Ordering::SeqCst);
    }

    #[doc(hidden)]
    pub fn transaction_count_for_tests(&self) -> usize {
        self.transaction_count.load(Ordering::Relaxed)
    }
}

fn is_content_hash_reservation_conflict(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("reserve content hashes failed")
        && message.contains("when executing against relation 'kb_content_hashes'")
        && message.contains("key exists in database")
}

fn is_retryable_content_hash_batch_lock(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    (message.contains("check batch content hashes failed")
        || message.contains("reserve content hashes failed"))
        && archon_cozo::is_retryable_cozo_error(&message)
}

fn pending_missing_chunks<'a>(
    batch: &[PendingChunk<'a>],
    existing: &HashSet<String>,
) -> Vec<PendingChunk<'a>> {
    batch
        .iter()
        .filter(|chunk| !existing.contains(&chunk.content_hash))
        .map(|chunk| PendingChunk {
            chunk: chunk.chunk,
            content_hash: chunk.content_hash.clone(),
            chunk_index: chunk.chunk_index,
            embedding: chunk.embedding,
        })
        .collect()
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
