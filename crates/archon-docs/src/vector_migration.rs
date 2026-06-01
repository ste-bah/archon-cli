use std::collections::BTreeMap;

use anyhow::{Context, Result};
use cozo::{DataValue, DbInstance, ScriptMutability, Vector};

use crate::vector_store::{DocVectorStore, VectorWrite};

#[derive(Clone, Debug, Default)]
pub struct VectorMigrationReport {
    pub scanned: usize,
    pub migrated: usize,
    pub skipped_existing: usize,
    pub last_chunk_id: Option<String>,
}

struct LegacyVectorRow {
    chunk_id: String,
    content_hash: String,
    provider: String,
    embedding: Vec<f32>,
}

pub fn migrate_legacy_vectors(
    db: &DbInstance,
    store: &DocVectorStore,
    limit: Option<usize>,
    batch_size: usize,
    after: Option<&str>,
) -> Result<VectorMigrationReport> {
    let rows = legacy_vector_rows(db, limit, after)?;
    let mut report = VectorMigrationReport {
        scanned: rows.len(),
        ..VectorMigrationReport::default()
    };
    let batch_size = batch_size.max(1);
    for batch in rows.chunks(batch_size) {
        let mut writes = Vec::with_capacity(batch.len());
        for row in batch {
            report.last_chunk_id = Some(row.chunk_id.clone());
            if store.has_vector(&row.provider, &row.chunk_id)? {
                report.skipped_existing += 1;
                continue;
            }
            writes.push(VectorWrite {
                chunk_id: &row.chunk_id,
                content_hash: &row.content_hash,
                provider: &row.provider,
                embedding: &row.embedding,
            });
        }
        report.migrated += store.put_vectors(&writes)?;
    }
    Ok(report)
}

fn legacy_vector_rows(
    db: &DbInstance,
    limit: Option<usize>,
    after: Option<&str>,
) -> Result<Vec<LegacyVectorRow>> {
    let mut params = BTreeMap::new();
    let mut predicates = Vec::new();
    if let Some(after) = after {
        params.insert("after".into(), DataValue::from(after));
        predicates.push("chunk_id > $after");
    }
    let mut script = "?[chunk_id, content_hash, provider, embedding] := \
         *vec_text_chunks{chunk_id, embedding, provider}, \
         *doc_chunks{chunk_id, content_hash}"
        .to_string();
    for predicate in predicates {
        script.push_str(", ");
        script.push_str(predicate);
    }
    script.push_str(" :order chunk_id");
    if let Some(limit) = limit {
        script.push_str(&format!(" :limit {}", limit.max(1)));
    }
    let result = crate::cozo_retry::run_script_guarded(
        db,
        &script,
        params,
        ScriptMutability::Immutable,
        "list legacy Cozo vectors",
    )
    .map_err(|e| anyhow::anyhow!("list legacy Cozo vectors failed: {e}"))?;
    result
        .rows
        .iter()
        .map(|row| row_from_data(row))
        .collect::<Result<Vec<_>>>()
}

fn row_from_data(row: &[DataValue]) -> Result<LegacyVectorRow> {
    let chunk_id = row[0].get_str().unwrap_or("").to_string();
    let content_hash = row[1].get_str().unwrap_or("").to_string();
    let provider = row[2].get_str().unwrap_or("").to_string();
    let embedding = match &row[3] {
        DataValue::Vec(Vector::F32(array)) => array.to_vec(),
        DataValue::Vec(_) => anyhow::bail!("legacy vector for {chunk_id} has wrong dtype"),
        _ => anyhow::bail!("legacy vector for {chunk_id} is not vector data"),
    };
    anyhow::ensure!(!chunk_id.is_empty(), "legacy vector row has empty chunk id");
    anyhow::ensure!(
        !provider.is_empty(),
        "legacy vector {chunk_id} has empty provider"
    );
    Ok(LegacyVectorRow {
        chunk_id,
        content_hash,
        provider,
        embedding,
    })
}

pub fn legacy_vector_count(db: &DbInstance) -> Result<usize> {
    crate::store::count_embeddings(db).context("count legacy Cozo vectors")
}

#[cfg(test)]
mod tests {
    use cozo::DbInstance;

    use super::*;
    use crate::models::ChunkArtifact;
    use crate::schema::{ensure_doc_schema, ensure_vec_schema};
    use crate::store;

    #[test]
    fn migrates_legacy_cozo_vectors_to_rocksdb() {
        let db = DbInstance::new("mem", "", Default::default()).unwrap();
        ensure_doc_schema(&db).unwrap();
        ensure_vec_schema(&db, 2).unwrap();
        let chunk = ChunkArtifact {
            chunk_id: "chunk-a".into(),
            document_id: "doc-a".into(),
            artifact_id: "artifact-a".into(),
            chunk_index: 0,
            page_start: 1,
            page_end: 1,
            content: "hello".into(),
            content_hash: "hash-a".into(),
            embedding_status: "indexed".into(),
        };
        store::insert_chunk(&db, &chunk).unwrap();
        store::insert_chunk_embedding(&db, "chunk-a", &[0.1, 0.9], "test").unwrap();
        let temp = tempfile::tempdir().unwrap();
        let vector_store = DocVectorStore::open(temp.path()).unwrap();
        let report = migrate_legacy_vectors(&db, &vector_store, None, 64, None).unwrap();
        assert_eq!(report.migrated, 1);
        assert_eq!(vector_store.count_vectors(Some("test")).unwrap(), 1);
    }
}
