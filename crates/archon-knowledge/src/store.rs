use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::errors::{KnowledgeError, Result};
use crate::schema::{
    ClaimPolarity, ClaimRecord, ContradictionRecord, EntityRecord, RelationRecord,
    SourceQualityRecord,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentChunk {
    pub chunk_id: String,
    pub document_id: String,
    pub content: String,
    pub content_hash: String,
}

pub fn list_doc_chunks(db: &DbInstance) -> Result<Vec<DocumentChunk>> {
    let script = r#"
        ?[chunk_id, document_id, content, content_hash] :=
            *doc_chunks{chunk_id, document_id, content, content_hash}
    "#;
    match db.run_script(script, Default::default(), ScriptMutability::Immutable) {
        Ok(result) => Ok(result
            .rows
            .iter()
            .map(|row| row_to_doc_chunk(row))
            .collect()),
        Err(e) if relation_missing(&e.to_string()) => Ok(Vec::new()),
        Err(e) => Err(KnowledgeError::Store(format!(
            "list doc_chunks failed: {e}"
        ))),
    }
}

pub fn insert_claim(db: &DbInstance, claim: &ClaimRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(claim.claim_id.as_str()));
    params.insert("cid".into(), DataValue::from(claim.chunk_id.as_str()));
    params.insert("did".into(), DataValue::from(claim.document_id.as_str()));
    params.insert("txt".into(), DataValue::from(claim.text.as_str()));
    params.insert(
        "subj".into(),
        DataValue::from(claim.normalized_subject.as_str()),
    );
    params.insert(
        "pred".into(),
        DataValue::from(claim.normalized_predicate.as_str()),
    );
    params.insert("pol".into(), DataValue::from(claim.polarity.as_str()));
    params.insert("conf".into(), DataValue::from(claim.confidence));
    params.insert("ts".into(), DataValue::from(claim.created_at.as_str()));
    db.run_script(
        r#"
        ?[claim_id, chunk_id, document_id, text, normalized_subject, normalized_predicate, polarity, confidence, created_at]
            <- [[$id, $cid, $did, $txt, $subj, $pred, $pol, $conf, $ts]]
        :put kb_claims {
            claim_id => chunk_id, document_id, text, normalized_subject,
            normalized_predicate, polarity, confidence, created_at
        }
        "#,
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| KnowledgeError::Store(format!("insert claim failed: {e}")))?;
    Ok(())
}

pub fn insert_entity(db: &DbInstance, entity: &EntityRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(entity.entity_id.as_str()));
    params.insert("name".into(), DataValue::from(entity.name.as_str()));
    params.insert("typ".into(), DataValue::from(entity.entity_type.as_str()));
    params.insert(
        "cid".into(),
        DataValue::from(entity.source_chunk_id.as_str()),
    );
    params.insert("mentions".into(), DataValue::from(entity.mentions));
    params.insert("conf".into(), DataValue::from(entity.confidence));
    params.insert("ts".into(), DataValue::from(entity.created_at.as_str()));
    db.run_script(
        r#"
        ?[entity_id, name, entity_type, source_chunk_id, mentions, confidence, created_at]
            <- [[$id, $name, $typ, $cid, $mentions, $conf, $ts]]
        :put kb_entities {
            entity_id => name, entity_type, source_chunk_id, mentions, confidence, created_at
        }
        "#,
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| KnowledgeError::Store(format!("insert entity failed: {e}")))?;
    Ok(())
}

pub fn insert_relation(db: &DbInstance, relation: &RelationRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(relation.relation_id.as_str()));
    params.insert(
        "src".into(),
        DataValue::from(relation.source_entity_id.as_str()),
    );
    params.insert(
        "dst".into(),
        DataValue::from(relation.target_entity_id.as_str()),
    );
    params.insert(
        "typ".into(),
        DataValue::from(relation.relation_type.as_str()),
    );
    params.insert(
        "cid".into(),
        DataValue::from(relation.source_chunk_id.as_str()),
    );
    params.insert("conf".into(), DataValue::from(relation.confidence));
    params.insert("ts".into(), DataValue::from(relation.created_at.as_str()));
    db.run_script(
        r#"
        ?[relation_id, source_entity_id, target_entity_id, relation_type, source_chunk_id, confidence, created_at]
            <- [[$id, $src, $dst, $typ, $cid, $conf, $ts]]
        :put kb_relations {
            relation_id => source_entity_id, target_entity_id, relation_type,
            source_chunk_id, confidence, created_at
        }
        "#,
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| KnowledgeError::Store(format!("insert relation failed: {e}")))?;
    Ok(())
}

pub fn upsert_source_quality(db: &DbInstance, record: &SourceQualityRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert("sid".into(), DataValue::from(record.source_id.as_str()));
    params.insert("score".into(), DataValue::from(record.score));
    params.insert("obs".into(), DataValue::from(record.observations));
    params.insert("out".into(), DataValue::from(record.last_outcome.as_str()));
    params.insert("ts".into(), DataValue::from(record.updated_at.as_str()));
    db.run_script(
        r#"
        ?[source_id, score, observations, last_outcome, updated_at]
            <- [[$sid, $score, $obs, $out, $ts]]
        :put kb_source_quality { source_id => score, observations, last_outcome, updated_at }
        "#,
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| KnowledgeError::Store(format!("upsert source quality failed: {e}")))?;
    Ok(())
}

pub fn insert_contradiction(db: &DbInstance, contradiction: &ContradictionRecord) -> Result<()> {
    let mut params = BTreeMap::new();
    params.insert(
        "id".into(),
        DataValue::from(contradiction.contradiction_id.as_str()),
    );
    params.insert(
        "left".into(),
        DataValue::from(contradiction.left_claim_id.as_str()),
    );
    params.insert(
        "right".into(),
        DataValue::from(contradiction.right_claim_id.as_str()),
    );
    params.insert(
        "typ".into(),
        DataValue::from(contradiction.contradiction_type.as_str()),
    );
    params.insert(
        "exp".into(),
        DataValue::from(contradiction.explanation.as_str()),
    );
    params.insert("conf".into(), DataValue::from(contradiction.confidence));
    params.insert(
        "ts".into(),
        DataValue::from(contradiction.created_at.as_str()),
    );
    db.run_script(
        r#"
        ?[contradiction_id, left_claim_id, right_claim_id, contradiction_type, explanation, confidence, created_at]
            <- [[$id, $left, $right, $typ, $exp, $conf, $ts]]
        :put kb_contradictions {
            contradiction_id => left_claim_id, right_claim_id, contradiction_type,
            explanation, confidence, created_at
        }
        "#,
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| KnowledgeError::Store(format!("insert contradiction failed: {e}")))?;
    Ok(())
}

pub fn list_claims(db: &DbInstance) -> Result<Vec<ClaimRecord>> {
    let result = db
        .run_script(
            r#"
            ?[claim_id, chunk_id, document_id, text, normalized_subject, normalized_predicate, polarity, confidence, created_at] :=
                *kb_claims{claim_id, chunk_id, document_id, text, normalized_subject, normalized_predicate, polarity, confidence, created_at}
            "#,
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| KnowledgeError::Store(format!("list claims failed: {e}")))?;
    Ok(result.rows.iter().map(|row| row_to_claim(row)).collect())
}

pub fn list_entities(db: &DbInstance) -> Result<Vec<EntityRecord>> {
    let result = db
        .run_script(
            r#"
            ?[entity_id, name, entity_type, source_chunk_id, mentions, confidence, created_at] :=
                *kb_entities{entity_id, name, entity_type, source_chunk_id, mentions, confidence, created_at}
            "#,
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| KnowledgeError::Store(format!("list entities failed: {e}")))?;
    Ok(result.rows.iter().map(|row| row_to_entity(row)).collect())
}

pub fn list_relations(db: &DbInstance) -> Result<Vec<RelationRecord>> {
    let result = db
        .run_script(
            r#"
            ?[relation_id, source_entity_id, target_entity_id, relation_type, source_chunk_id, confidence, created_at] :=
                *kb_relations{relation_id, source_entity_id, target_entity_id, relation_type, source_chunk_id, confidence, created_at}
            "#,
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| KnowledgeError::Store(format!("list relations failed: {e}")))?;
    Ok(result.rows.iter().map(|row| row_to_relation(row)).collect())
}

pub fn list_source_quality(db: &DbInstance) -> Result<Vec<SourceQualityRecord>> {
    let result = db
        .run_script(
            r#"
            ?[source_id, score, observations, last_outcome, updated_at] :=
                *kb_source_quality{source_id, score, observations, last_outcome, updated_at}
            "#,
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| KnowledgeError::Store(format!("list source quality failed: {e}")))?;
    Ok(result
        .rows
        .iter()
        .map(|row| row_to_source_quality(row))
        .collect())
}

pub fn list_contradictions(db: &DbInstance) -> Result<Vec<ContradictionRecord>> {
    let result = db
        .run_script(
            r#"
            ?[contradiction_id, left_claim_id, right_claim_id, contradiction_type, explanation, confidence, created_at] :=
                *kb_contradictions{contradiction_id, left_claim_id, right_claim_id, contradiction_type, explanation, confidence, created_at}
            "#,
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| KnowledgeError::Store(format!("list contradictions failed: {e}")))?;
    Ok(result
        .rows
        .iter()
        .map(|row| row_to_contradiction(row))
        .collect())
}

pub fn get_source_quality(db: &DbInstance, source_id: &str) -> Result<Option<SourceQualityRecord>> {
    let mut params = BTreeMap::new();
    params.insert("sid".into(), DataValue::from(source_id));
    let result = db
        .run_script(
            r#"
            ?[source_id, score, observations, last_outcome, updated_at] :=
                *kb_source_quality{source_id, score, observations, last_outcome, updated_at},
                source_id = $sid
            "#,
            params,
            ScriptMutability::Immutable,
        )
        .map_err(|e| KnowledgeError::Store(format!("get source quality failed: {e}")))?;
    Ok(result.rows.first().map(|row| row_to_source_quality(row)))
}

fn row_to_doc_chunk(row: &[DataValue]) -> DocumentChunk {
    DocumentChunk {
        chunk_id: str_col(row, 0),
        document_id: str_col(row, 1),
        content: str_col(row, 2),
        content_hash: str_col(row, 3),
    }
}

fn row_to_claim(row: &[DataValue]) -> ClaimRecord {
    ClaimRecord {
        claim_id: str_col(row, 0),
        chunk_id: str_col(row, 1),
        document_id: str_col(row, 2),
        text: str_col(row, 3),
        normalized_subject: str_col(row, 4),
        normalized_predicate: str_col(row, 5),
        polarity: ClaimPolarity::from_str(&str_col(row, 6)),
        confidence: float_col(row, 7),
        created_at: str_col(row, 8),
    }
}

fn row_to_entity(row: &[DataValue]) -> EntityRecord {
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

fn row_to_relation(row: &[DataValue]) -> RelationRecord {
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

fn row_to_source_quality(row: &[DataValue]) -> SourceQualityRecord {
    SourceQualityRecord {
        source_id: str_col(row, 0),
        score: float_col(row, 1),
        observations: int_col(row, 2),
        last_outcome: str_col(row, 3),
        updated_at: str_col(row, 4),
    }
}

fn row_to_contradiction(row: &[DataValue]) -> ContradictionRecord {
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

fn relation_missing(message: &str) -> bool {
    message.contains("Cannot find requested stored relation")
        || message.contains("not found")
        || message.contains("does not exist")
}
