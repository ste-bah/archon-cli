//! CozoDB persistence for file-level index replacements.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, DbInstance, ScriptMutability, Vector};

use crate::metadata::CodeChunk;

fn cozo_error(context: &str, error: cozo::Error) -> anyhow::Error {
    anyhow::anyhow!("{context}: {error}")
}

pub(super) struct PreparedChunk {
    pub(super) chunk: CodeChunk,
    pub(super) embedding: Vec<f32>,
}

pub(super) enum ReplaceFileOutcome {
    Committed,
    Cancelled,
}

pub(super) struct FileStore<'a> {
    db: &'a DbInstance,
    dimension: usize,
    guard: &'a archon_cozo::CozoGuardConfig,
}

impl<'a> FileStore<'a> {
    pub(super) fn new(
        db: &'a DbInstance,
        dimension: usize,
        guard: &'a archon_cozo::CozoGuardConfig,
    ) -> Self {
        Self {
            db,
            dimension,
            guard,
        }
    }

    pub(super) fn ensure_schema(&self) -> Result<()> {
        self.run_idempotent(&format!(
            ":create code_chunks {{
                chunk_id: String
                =>
                file_path: String,
                language: String,
                line_start: Int,
                line_end: Int,
                chunk_content: String,
                file_hash: String,
                indexed_at: Float,
                embedding: <F32; {}>
            }}",
            self.dimension
        ))?;
        self.run_idempotent(":create file_states { file_path: String => file_hash: String }")?;
        self.run_idempotent(&format!(
            "::hnsw create code_chunks:chunk_embedding_idx {{
                dim: {},
                m: 50,
                dtype: F32,
                fields: [embedding],
                distance: Cosine,
                ef_construction: 200
            }}",
            self.dimension
        ))
    }

    /// Is `file_path` already indexed at exactly `file_hash`?
    ///
    /// Guarded for the same reason the writes below are. `.archon/leann.db` is
    /// per-working-directory, so two archon processes in one repository -- a
    /// TUI session and a dashboard, or two terminals -- contend by
    /// construction. This check runs first for *every* file in the walk, so an
    /// unguarded SQLITE_BUSY here ended the whole pass before the guarded
    /// writes ever got their turn to back off: the loser of the race abandoned
    /// its index until the next session start.
    ///
    /// Immutable, so the guard retries without taking the write lock -- a read
    /// has no reason to serialise against other readers.
    ///
    /// Exhausting the retry budget returns `true`, not `Err`. Both callers read
    /// `true` as "already current, skip it", so a file that stays contended
    /// past the budget costs that one file rather than the walk, and the next
    /// pass picks it up again because nothing was written for it. Propagating
    /// the error instead would reinstate exactly the abandon-everything
    /// behaviour this guard exists to prevent.
    pub(super) fn file_hash_matches(&self, file_path: &str, file_hash: &str) -> Result<bool> {
        let mut params = BTreeMap::new();
        params.insert("fp".to_string(), DataValue::from(file_path));
        params.insert("fh".to_string(), DataValue::from(file_hash));
        let result = archon_cozo::run_script_guarded(
            self.db,
            "?[file_path] := *file_states{file_path, file_hash}, file_path = $fp, file_hash = $fh",
            params,
            ScriptMutability::Immutable,
            "leann index: file state hash check",
            self.guard,
        );
        match result {
            Ok(rows) => Ok(!rows.rows.is_empty()),
            Err(error) => {
                let message = format!("{error:#}");
                if archon_cozo::is_retryable_cozo_error(&message) {
                    tracing::warn!(
                        file_path,
                        error = %message,
                        "LEANN index: file state check still busy after the retry budget; \
                         skipping this file for this pass"
                    );
                    return Ok(true);
                }
                Err(anyhow::anyhow!("file state hash check query: {message}"))
            }
        }
    }

    /// Replace every chunk of `file_path` in one Cozo multi-transaction.
    ///
    /// This is a write path but not a `run_script(.., Mutable)` one, so it is
    /// wrapped in [`archon_cozo::run_guarded`] as a whole: the transaction is
    /// atomic, which makes the guard's retry-on-SQLITE_BUSY loop safe to apply
    /// to the entire body.
    pub(super) fn replace_file_with_cancel<F>(
        &self,
        file_path: &str,
        file_hash: &str,
        chunks: &[PreparedChunk],
        cancelled: F,
    ) -> Result<ReplaceFileOutcome>
    where
        F: Fn() -> bool,
    {
        archon_cozo::run_guarded(
            "leann index: replace indexed file",
            ScriptMutability::Mutable,
            self.guard,
            || {
                let transaction = self.db.multi_transaction(true);
                let result =
                    self.replace_file_in_transaction(&transaction, file_path, file_hash, chunks);
                match result {
                    Ok(()) if cancelled() => {
                        let _ = transaction.abort();
                        Ok(ReplaceFileOutcome::Cancelled)
                    }
                    Ok(()) => transaction
                        .commit()
                        .map(|()| ReplaceFileOutcome::Committed)
                        .map_err(|error| cozo_error("commit file replacement", error)),
                    Err(error) => {
                        let _ = transaction.abort();
                        Err(error)
                    }
                }
            },
        )
    }

    pub(super) fn remove_file(&self, file_path: &str) -> Result<()> {
        archon_cozo::run_guarded(
            "leann index: remove indexed file",
            ScriptMutability::Mutable,
            self.guard,
            || {
                let transaction = self.db.multi_transaction(true);
                let result = (|| {
                    remove_file_chunks_in_transaction(&transaction, file_path)?;
                    let mut params = BTreeMap::new();
                    params.insert("fp".to_string(), DataValue::from(file_path));
                    transaction
                        .run_script(
                            "?[file_path] <- [[$fp]] :rm file_states { file_path }",
                            params,
                        )
                        .map_err(|error| cozo_error("remove file state", error))?;
                    Ok(())
                })();
                match result {
                    Ok(()) => transaction
                        .commit()
                        .map_err(|error| cozo_error("commit file removal", error)),
                    Err(error) => {
                        let _ = transaction.abort();
                        Err(error)
                    }
                }
            },
        )
    }

    #[cfg(test)]
    pub(super) fn remove_file_chunks(&self, file_path: &str) -> Result<()> {
        let transaction = self.db.multi_transaction(true);
        let result = remove_file_chunks_in_transaction(&transaction, file_path);
        match result {
            Ok(()) => transaction
                .commit()
                .map_err(|error| cozo_error("commit chunk removal", error)),
            Err(error) => {
                let _ = transaction.abort();
                Err(error)
            }
        }
    }

    fn replace_file_in_transaction(
        &self,
        transaction: &cozo::MultiTransaction,
        file_path: &str,
        file_hash: &str,
        chunks: &[PreparedChunk],
    ) -> Result<()> {
        remove_file_chunks_in_transaction(transaction, file_path)?;
        put_chunks_in_transaction(transaction, chunks)?;
        let mut params = BTreeMap::new();
        params.insert("fp".to_string(), DataValue::from(file_path));
        params.insert("fh".to_string(), DataValue::from(file_hash));
        transaction
            .run_script(
                "?[file_path, file_hash] <- [[$fp, $fh]] :put file_states { file_path => file_hash }",
                params,
            )
            .map_err(|error| cozo_error("update file state", error))?;
        Ok(())
    }

    fn run_idempotent(&self, script: &str) -> Result<()> {
        match archon_cozo::run_script_guarded(
            self.db,
            script,
            Default::default(),
            ScriptMutability::Mutable,
            "leann index schema: create relation or index",
            self.guard,
        ) {
            Ok(_) => Ok(()),
            Err(error) => {
                let message = error.to_string();
                if message.contains("already exists")
                    || message.contains("conflicts")
                    || message.contains("index with the same name")
                {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("CozoDB schema script failed: {message}"))
                }
            }
        }
    }
}

#[cfg(test)]
mod file_hash_matches_tests {
    use super::*;

    fn store_db() -> DbInstance {
        DbInstance::new("mem", "", Default::default()).expect("in-memory CozoDB")
    }

    #[test]
    fn guarding_the_read_leaves_the_answer_unchanged() {
        // The guard wraps the query; it must not alter what the query means.
        let db = store_db();
        let guard = archon_cozo::CozoGuardConfig::default();
        let store = FileStore::new(&db, 8, &guard);
        store.ensure_schema().expect("schema");

        assert!(
            !store.file_hash_matches("src/lib.rs", "abc").expect("read"),
            "an unindexed file has no stored hash to match"
        );

        store
            .replace_file_with_cancel("src/lib.rs", "abc", &[], || false)
            .expect("write");

        assert!(
            store.file_hash_matches("src/lib.rs", "abc").expect("read"),
            "the stored hash matches"
        );
        assert!(
            !store.file_hash_matches("src/lib.rs", "def").expect("read"),
            "a changed hash does not match"
        );
    }

    #[test]
    fn the_observed_busy_error_routes_to_skip_not_failure() {
        // Verbatim from the issue #140 report. If Cozo ever reworded this, the
        // skip branch would go unreachable and the walk would start aborting
        // again on contention -- silently, because the symptom is a log line in
        // a session file. Pin the string that has to keep classifying.
        assert!(archon_cozo::is_retryable_cozo_error(
            "file state hash check query: database is locked (code 5)"
        ));
    }
}

fn remove_file_chunks_in_transaction(
    transaction: &cozo::MultiTransaction,
    file_path: &str,
) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("fp".to_string(), DataValue::from(file_path));
    transaction
        .run_script(
            "?[chunk_id, file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding] :=
             *code_chunks{chunk_id, file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding},
             file_path = $fp
             :rm code_chunks { chunk_id => file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding }",
            params,
        )
        .map_err(|error| cozo_error("remove prior file chunks", error))?;
    Ok(())
}

fn put_chunks_in_transaction(
    transaction: &cozo::MultiTransaction,
    chunks: &[PreparedChunk],
) -> Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }

    let indexed_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    let rows = chunks
        .iter()
        .map(|prepared| {
            let metadata = &prepared.chunk.metadata;
            DataValue::List(vec![
                DataValue::from(uuid::Uuid::new_v4().to_string()),
                DataValue::from(metadata.file_path.to_string_lossy().as_ref()),
                DataValue::from(metadata.language.as_str()),
                DataValue::from(metadata.line_start as i64),
                DataValue::from(metadata.line_end as i64),
                DataValue::from(metadata.chunk_content.as_str()),
                DataValue::from(metadata.file_hash.as_str()),
                DataValue::from(indexed_at),
                DataValue::Vec(Vector::F32(ndarray::Array1::from_vec(
                    prepared.embedding.clone(),
                ))),
            ])
        })
        .collect();
    let mut params = BTreeMap::new();
    params.insert("rows".to_string(), DataValue::List(rows));
    transaction
        .run_script(
            "?[chunk_id, file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding] <- $rows
             :put code_chunks { chunk_id => file_path, language, line_start, line_end, chunk_content, file_hash, indexed_at, embedding }",
            params,
        )
        .map_err(|error| cozo_error("insert replacement chunks", error))?;
    Ok(())
}
