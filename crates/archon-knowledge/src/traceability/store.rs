//! Persisting requirement entities and anchored edges in CozoDB.
//!
//! # Why a side relation rather than four more columns on `kb_relations`
//!
//! An anchor edge *is* a [`RelationRecord`] — requirement entity to code
//! citation — and it is stored as one. But `RelationRecord` is a flat struct
//! with no metadata field, and its Cozo schema is hand-mirrored to it, so
//! carrying `file_path`, `line_start`, `line_end` and `file_hash` on the edge
//! means widening `kb_relations`.
//!
//! That would break every existing database. [`crate::schema::ensure_knowledge_schema`]
//! swallows "already exists", so an on-disk store keeps the columns it was
//! created with; a widened `:put` against it fails on an unknown column, and the
//! failure would land on knowledge writes that have nothing to do with
//! traceability. `kb_requirement_anchors` is new, so its `:create` succeeds on
//! old and new stores alike, and it joins to the edge on `relation_id`.
//!
//! The edge still carries its citation directly — `source_chunk_id` and
//! `target_entity_id` are both `path:start-end` — so a reader of `kb_relations`
//! alone still sees a `file:line` rather than an unqualified claim. The side
//! relation adds the hash that makes the edge invalidatable and the level that
//! says what it proved.

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::errors::{KnowledgeError, Result};
use crate::schema::{RelationRecord, run_create};

use super::anchors::{Anchor, anchor_relation_id};
use super::ladder::ProofLevel;

pub const KB_REQUIREMENT_ANCHORS_SCHEMA: &str = r#":create kb_requirement_anchors {
    relation_id: String =>
    requirement_id: String,
    task_id: String,
    file_path: String,
    line_start: Int,
    line_end: Int,
    file_hash: String,
    proof_level: String,
    created_at: String
}"#;

/// Create the traceability side relation. Idempotent.
pub fn ensure_traceability_schema(db: &DbInstance) -> Result<()> {
    run_create(db, KB_REQUIREMENT_ANCHORS_SCHEMA)
}

/// The anchor detail hanging off one `kb_relations` edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnchorRecord {
    /// Joins to `kb_relations.relation_id`.
    pub relation_id: String,
    pub requirement_id: String,
    pub task_id: String,
    pub file_path: String,
    pub line_start: i64,
    pub line_end: i64,
    /// SHA-256 of the whole file at anchor time. Without this the edge could
    /// never be known-stale, only silently wrong.
    pub file_hash: String,
    pub proof_level: String,
    pub created_at: String,
}

impl AnchorRecord {
    pub fn from_anchor(anchor: &Anchor, level: ProofLevel, created_at: &str) -> Self {
        Self {
            relation_id: anchor_relation_id(anchor),
            requirement_id: anchor.requirement_id.clone(),
            task_id: anchor.task_id.clone(),
            file_path: anchor.file_path.clone(),
            line_start: anchor.line_start as i64,
            line_end: anchor.line_end as i64,
            file_hash: anchor.file_hash.clone(),
            proof_level: level.as_str().to_string(),
            created_at: created_at.to_string(),
        }
    }

    pub fn citation(&self) -> String {
        format!("{}:{}-{}", self.file_path, self.line_start, self.line_end)
    }
}

/// Write the edge and its anchor detail.
///
/// The relation goes in first: an anchor row that joins to nothing would be a
/// dangling citation, which is the failure mode the whole design exists to
/// prevent.
pub fn insert_anchor(
    db: &DbInstance,
    relation: &RelationRecord,
    anchor: &AnchorRecord,
) -> Result<()> {
    crate::store::insert_relation(db, relation)?;

    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(anchor.relation_id.as_str()));
    params.insert(
        "req".into(),
        DataValue::from(anchor.requirement_id.as_str()),
    );
    params.insert("task".into(), DataValue::from(anchor.task_id.as_str()));
    params.insert("path".into(), DataValue::from(anchor.file_path.as_str()));
    params.insert("ls".into(), DataValue::from(anchor.line_start));
    params.insert("le".into(), DataValue::from(anchor.line_end));
    params.insert("hash".into(), DataValue::from(anchor.file_hash.as_str()));
    params.insert("level".into(), DataValue::from(anchor.proof_level.as_str()));
    params.insert("ts".into(), DataValue::from(anchor.created_at.as_str()));

    archon_cozo::run_bound_script_guarded(
        db,
        r#"
        ?[relation_id, requirement_id, task_id, file_path, line_start, line_end, file_hash, proof_level, created_at]
            <- [[$id, $req, $task, $path, $ls, $le, $hash, $level, $ts]]
        :put kb_requirement_anchors {
            relation_id => requirement_id, task_id, file_path, line_start, line_end,
            file_hash, proof_level, created_at
        }
        "#,
        params,
        ScriptMutability::Mutable,
        "traceability store: insert kb_requirement_anchors row",
    )
    .map_err(|e| KnowledgeError::Store(format!("insert requirement anchor failed: {e}")))?;
    Ok(())
}

/// Every recorded anchor. Missing relation is an empty list, not an error: a
/// store that has never been traced has no anchors, which is a true answer.
pub fn list_anchors(db: &DbInstance) -> Result<Vec<AnchorRecord>> {
    let script = r#"
        ?[relation_id, requirement_id, task_id, file_path, line_start, line_end, file_hash, proof_level, created_at] :=
            *kb_requirement_anchors{relation_id, requirement_id, task_id, file_path, line_start, line_end, file_hash, proof_level, created_at}
    "#;
    match db.run_script(script, Default::default(), ScriptMutability::Immutable) {
        Ok(result) => Ok(result
            .rows
            .iter()
            .map(|row| row_to_anchor(row.as_slice()))
            .collect()),
        Err(e) if crate::store::relation_missing(&e.to_string()) => Ok(Vec::new()),
        Err(e) => Err(KnowledgeError::Store(format!(
            "list requirement anchors failed: {e}"
        ))),
    }
}

/// Recorded anchors for one requirement.
pub fn anchors_for(db: &DbInstance, requirement_id: &str) -> Result<Vec<AnchorRecord>> {
    Ok(list_anchors(db)?
        .into_iter()
        .filter(|record| record.requirement_id == requirement_id)
        .collect())
}

/// Positional decoding, mirroring the `?[...]` head above. Total by design: a
/// partially decodable row is worth more to a caller than a failed query.
fn row_to_anchor(row: &[DataValue]) -> AnchorRecord {
    let text = |idx: usize| {
        row.get(idx)
            .and_then(DataValue::get_str)
            .unwrap_or("")
            .to_string()
    };
    let int = |idx: usize| row.get(idx).and_then(DataValue::get_int).unwrap_or(0);
    AnchorRecord {
        relation_id: text(0),
        requirement_id: text(1),
        task_id: text(2),
        file_path: text(3),
        line_start: int(4),
        line_end: int(5),
        file_hash: text(6),
        proof_level: text(7),
        created_at: text(8),
    }
}

#[cfg(test)]
mod tests;
