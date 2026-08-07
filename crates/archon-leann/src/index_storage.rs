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

/// What the walk should do with a file, after asking the store about it.
///
/// Three states rather than a bool because the third one used to be encoded as
/// `true` — "pretend it is current" — and that reads identically to a genuinely
/// up-to-date file at both call sites. The distinction matters twice: a
/// contended file must be counted as skipped so the pass does not report itself
/// complete, and it must leave `file_states` untouched so the next pass sees it
/// as stale and retries it.
pub(super) enum FileState {
    /// Stored at this exact hash; nothing to do.
    Current,
    /// Absent or stored at a different hash; needs indexing.
    Stale,
    /// The store stayed busy past the retry budget; unknown, try next pass.
    Contended,
}

/// Turn "another process held the store" into a skipped file, not a dead pass.
///
/// This is issue #140. [`FileStore::replace_file_with_cancel`] is the guard's
/// longest critical section, and two indexers on one `.archon/leann.db` contend
/// by construction -- a TUI session and a dashboard, or two terminals. When the
/// loser exhausted its budget the error went out through `?` and unwound the
/// entire walk, discarding a pass that may already have persisted thousands of
/// files, and nothing retried until the next session start.
///
/// `Ok(None)` means skip this file. Nothing was committed for it, so its
/// `file_states` row stays stale and the next pass picks it up again -- which is
/// the whole reason skipping is safe. Recording it as indexed would be the worse
/// bug: the file would never be re-examined and search would be quietly
/// incomplete.
///
/// Only contention is absorbed. A malformed script, a missing relation or a
/// panic still propagates, because those do not get better on the next pass and
/// skipping past them would hide a real fault behind a warning.
pub(super) fn skip_if_contended(
    file_path: &str,
    outcome: Result<ReplaceFileOutcome>,
    stats: &mut crate::metadata::IndexStats,
) -> Result<Option<ReplaceFileOutcome>> {
    match outcome {
        Ok(outcome) => Ok(Some(outcome)),
        Err(error) => {
            let message = format!("{error:#}");
            if !archon_cozo::is_store_contention(&message) {
                return Err(error);
            }
            stats.skipped_files += 1;
            tracing::warn!(
                file_path,
                error = %message,
                "LEANN index: store still held by another process after the retry budget; \
                 skipping this file for this pass"
            );
            Ok(None)
        }
    }
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
    /// Exhausting the retry budget yields [`FileState::Contended`], not `Err`.
    /// A file that stays contended past the budget costs that one file rather
    /// than the walk, and the next pass picks it up again because nothing was
    /// written for it. Propagating the error instead would reinstate exactly
    /// the abandon-everything behaviour this guard exists to prevent.
    pub(super) fn file_state(&self, file_path: &str, file_hash: &str) -> Result<FileState> {
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
            Ok(rows) if rows.rows.is_empty() => Ok(FileState::Stale),
            Ok(_) => Ok(FileState::Current),
            Err(error) => {
                let message = format!("{error:#}");
                if archon_cozo::is_store_contention(&message) {
                    tracing::warn!(
                        file_path,
                        error = %message,
                        "LEANN index: file state check still busy after the retry budget; \
                         skipping this file for this pass"
                    );
                    return Ok(FileState::Contended);
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

    /// Run a `:create`/`::hnsw create` that another process may already have run.
    ///
    /// Deliberately not `run_script_guarded`: that folds the Cozo error into an
    /// `anyhow::Error` built from its `Display` alone, and `cozo::Error` is a
    /// `miette::Report` whose `Display` shows only the outermost context. When
    /// two processes race on a fresh database the already-exists detail is one
    /// link further down -- Cozo wraps it as `when executing against relation
    /// 'code_chunks'` -- so no rendering of the folded error, `{:#}` included,
    /// ever reaches it, and the benign match below saw a message it could not
    /// classify. That is issue #144. Rendering the whole chain here is what
    /// lets that match stay narrow: forgiving the wrapper text instead would
    /// have forgiven every malformed schema change too, since Cozo reports
    /// those through the same wrapper.
    fn run_idempotent(&self, script: &str) -> Result<()> {
        let result = archon_cozo::run_guarded(
            "leann index schema: create relation or index",
            ScriptMutability::Mutable,
            self.guard,
            || {
                self.db
                    .run_script(script, BTreeMap::new(), ScriptMutability::Mutable)
                    .map(|_| ())
                    .map_err(|error| anyhow::anyhow!("{}", archon_cozo::render_cozo_error(&error)))
            },
        );
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                let message = format!("{error:#}");
                if is_benign_schema_conflict(&message) {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("CozoDB schema script failed: {message}"))
                }
            }
        }
    }
}

/// Did the schema statement fail only because someone already applied it?
///
/// Each phrase names a specific Cozo diagnostic and nothing broader:
///
/// * `conflicts with an existing one` — the `:create` pre-check found the
///   relation, which is what a second process sees once the first has
///   committed.
/// * `already exists` — the same conflict caught inside the transaction
///   instead (`Cannot create relation X as one with the same name already
///   exists`), which is the racing shape, and the HNSW equivalent (`index X
///   for relation Y already exists`).
///
/// What is pointedly *not* here is Cozo's wrapper, `when executing against
/// relation 'X'`. It carries no evidence of what went wrong -- a bad column
/// type or an unparseable schema arrives wrapped in exactly the same words --
/// so matching it would silence real schema faults in the one place they most
/// need to be seen. Reaching the cause instead is [`archon_cozo::render_cozo_error`]'s
/// job.
fn is_benign_schema_conflict(message: &str) -> bool {
    message.contains("conflicts with an existing one")
        || message.contains("already exists")
        || message.contains("index with the same name")
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

#[cfg(test)]
#[path = "index_storage_tests.rs"]
mod index_storage_tests;
