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
}

impl<'a> FileStore<'a> {
    pub(super) fn new(db: &'a DbInstance, dimension: usize) -> Self {
        Self { db, dimension }
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

    pub(super) fn file_hash_matches(&self, file_path: &str, file_hash: &str) -> Result<bool> {
        let mut params = BTreeMap::new();
        params.insert("fp".to_string(), DataValue::from(file_path));
        params.insert("fh".to_string(), DataValue::from(file_hash));
        let result = self
            .db
            .run_script(
                "?[file_path] := *file_states{file_path, file_hash}, file_path = $fp, file_hash = $fh",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(|error| cozo_error("file state hash check query", error))?;
        Ok(!result.rows.is_empty())
    }

    pub(super) fn replace_file_with_cancel<F>(
        &self,
        file_path: &str,
        file_hash: &str,
        chunks: &[PreparedChunk],
        cancelled: F,
    ) -> Result<ReplaceFileOutcome>
    where
        F: FnOnce() -> bool,
    {
        let transaction = self.db.multi_transaction(true);
        let result = self.replace_file_in_transaction(&transaction, file_path, file_hash, chunks);
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
    }

    pub(super) fn remove_file(&self, file_path: &str) -> Result<()> {
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
        match self
            .db
            .run_script(script, Default::default(), ScriptMutability::Mutable)
        {
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
