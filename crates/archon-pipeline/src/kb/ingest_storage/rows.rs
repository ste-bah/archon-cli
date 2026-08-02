//! Building the `:put` scripts one batch of chunks turns into.
//!
//! Owns the three row builders that translate reserved [`PendingChunk`] values
//! into bound Cozo parameters and issue them against an open transaction: the
//! content-hash reservations, the `kb_nodes` rows, and the optional
//! `kb_embeddings` rows.
//!
//! They stay together because they share a row shape and a batch: all three are
//! driven from the same `&[(String, &PendingChunk)]` inside one transaction, so
//! a change to how a node id or a content hash is carried has to land in all of
//! them at once.

use std::collections::BTreeMap;

use anyhow::Result;
use cozo::{DataValue, MultiTransaction, Vector};
use ndarray::Array1;

use super::super::schema::KbNodeType;
use super::{ChunkStorage, PendingChunk};

impl ChunkStorage {
    pub(super) fn insert_hash_rows(
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

    pub(super) fn insert_node_rows(
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

    pub(super) fn insert_embedding_rows(
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
