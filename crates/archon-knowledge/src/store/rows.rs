//! Decoding Cozo rows into knowledge records.
//!
//! Owns one `row_to_*` function per relation the store reads, plus the column
//! readers they share. Every positional assumption about a query's projection
//! lives here: change a `?[...]` head in [`super`] and this is the file that
//! has to change with it.
//!
//! The column readers are total — a missing or wrongly typed column yields the
//! zero value rather than an error — because a partially decodable row is still
//! worth more to a caller than a failed query.

use cozo::DataValue;

use crate::schema::{
    ClaimPolarity, ClaimRecord, ContradictionRecord, EntityRecord, RelationRecord,
    SourceQualityRecord,
};

use super::DocumentChunk;

pub(super) fn row_to_doc_chunk(row: &[DataValue]) -> DocumentChunk {
    DocumentChunk {
        chunk_id: str_col(row, 0),
        document_id: str_col(row, 1),
        content: str_col(row, 2),
        content_hash: str_col(row, 3),
    }
}

pub(super) fn row_to_claim(row: &[DataValue]) -> ClaimRecord {
    ClaimRecord {
        claim_id: str_col(row, 0),
        chunk_id: str_col(row, 1),
        document_id: str_col(row, 2),
        text: str_col(row, 3),
        normalized_subject: str_col(row, 4),
        normalized_predicate: str_col(row, 5),
        polarity: ClaimPolarity::parse(&str_col(row, 6)),
        confidence: float_col(row, 7),
        created_at: str_col(row, 8),
    }
}

pub(super) fn row_to_entity(row: &[DataValue]) -> EntityRecord {
    EntityRecord {
        entity_id: str_col(row, 0),
        name: str_col(row, 1),
        entity_type: str_col(row, 2),
        source_chunk_id: str_col(row, 3),
        mentions: int_col(row, 4),
        confidence: float_col(row, 5),
        created_at: str_col(row, 6),
    }
}

pub(super) fn row_to_relation(row: &[DataValue]) -> RelationRecord {
    RelationRecord {
        relation_id: str_col(row, 0),
        source_entity_id: str_col(row, 1),
        target_entity_id: str_col(row, 2),
        relation_type: str_col(row, 3),
        source_chunk_id: str_col(row, 4),
        confidence: float_col(row, 5),
        created_at: str_col(row, 6),
    }
}

pub(super) fn row_to_source_quality(row: &[DataValue]) -> SourceQualityRecord {
    SourceQualityRecord {
        source_id: str_col(row, 0),
        score: float_col(row, 1),
        observations: int_col(row, 2),
        last_outcome: str_col(row, 3),
        updated_at: str_col(row, 4),
    }
}

pub(super) fn row_to_contradiction(row: &[DataValue]) -> ContradictionRecord {
    ContradictionRecord {
        contradiction_id: str_col(row, 0),
        left_claim_id: str_col(row, 1),
        right_claim_id: str_col(row, 2),
        contradiction_type: str_col(row, 3),
        explanation: str_col(row, 4),
        confidence: float_col(row, 5),
        created_at: str_col(row, 6),
    }
}

fn str_col(row: &[DataValue], idx: usize) -> String {
    row.get(idx)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

fn float_col(row: &[DataValue], idx: usize) -> f64 {
    row.get(idx).and_then(DataValue::get_float).unwrap_or(0.0)
}

fn int_col(row: &[DataValue], idx: usize) -> i64 {
    row.get(idx).and_then(DataValue::get_int).unwrap_or(0)
}
