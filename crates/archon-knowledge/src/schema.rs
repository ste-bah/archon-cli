use cozo::{DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::errors::{KnowledgeError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClaimPolarity {
    Positive,
    Negative,
}

impl ClaimPolarity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }

    pub fn from_str(value: &str) -> Self {
        if value == "negative" {
            Self::Negative
        } else {
            Self::Positive
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClaimRecord {
    pub claim_id: String,
    pub chunk_id: String,
    pub document_id: String,
    pub text: String,
    pub normalized_subject: String,
    pub normalized_predicate: String,
    pub polarity: ClaimPolarity,
    pub confidence: f64,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityRecord {
    pub entity_id: String,
    pub name: String,
    pub entity_type: String,
    pub source_chunk_id: String,
    pub mentions: i64,
    pub confidence: f64,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelationRecord {
    pub relation_id: String,
    pub source_entity_id: String,
    pub target_entity_id: String,
    pub relation_type: String,
    pub source_chunk_id: String,
    pub confidence: f64,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceQualityRecord {
    pub source_id: String,
    pub score: f64,
    pub observations: i64,
    pub last_outcome: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContradictionRecord {
    pub contradiction_id: String,
    pub left_claim_id: String,
    pub right_claim_id: String,
    pub contradiction_type: String,
    pub explanation: String,
    pub confidence: f64,
    pub created_at: String,
}

pub const KB_CLAIMS_SCHEMA: &str = r#":create kb_claims {
    claim_id: String =>
    chunk_id: String,
    document_id: String,
    text: String,
    normalized_subject: String,
    normalized_predicate: String,
    polarity: String,
    confidence: Float,
    created_at: String
}"#;

pub const KB_ENTITIES_SCHEMA: &str = r#":create kb_entities {
    entity_id: String =>
    name: String,
    entity_type: String,
    source_chunk_id: String,
    mentions: Int,
    confidence: Float,
    created_at: String
}"#;

pub const KB_RELATIONS_SCHEMA: &str = r#":create kb_relations {
    relation_id: String =>
    source_entity_id: String,
    target_entity_id: String,
    relation_type: String,
    source_chunk_id: String,
    confidence: Float,
    created_at: String
}"#;

pub const KB_SOURCE_QUALITY_SCHEMA: &str = r#":create kb_source_quality {
    source_id: String =>
    score: Float,
    observations: Int,
    last_outcome: String,
    updated_at: String
}"#;

pub const KB_CONTRADICTIONS_SCHEMA: &str = r#":create kb_contradictions {
    contradiction_id: String =>
    left_claim_id: String,
    right_claim_id: String,
    contradiction_type: String,
    explanation: String,
    confidence: Float,
    created_at: String
}"#;

pub fn ensure_knowledge_schema(db: &DbInstance) -> Result<()> {
    for script in [
        KB_CLAIMS_SCHEMA,
        KB_ENTITIES_SCHEMA,
        KB_RELATIONS_SCHEMA,
        KB_SOURCE_QUALITY_SCHEMA,
        KB_CONTRADICTIONS_SCHEMA,
    ] {
        run_create(db, script)?;
    }
    Ok(())
}

fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already exists") || msg.contains("conflicts with an existing") {
                Ok(())
            } else {
                Err(KnowledgeError::Schema(msg))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> DbInstance {
        DbInstance::new("mem", "", "").unwrap()
    }

    #[test]
    fn ensure_schema_creates_all_relations() {
        let db = test_db();
        ensure_knowledge_schema(&db).unwrap();
        let result = db
            .run_script(
                "::relations",
                Default::default(),
                ScriptMutability::Immutable,
            )
            .unwrap();
        let names: Vec<String> = result
            .rows
            .iter()
            .filter_map(|row| row.first().and_then(|v| v.get_str()).map(str::to_string))
            .collect();
        for expected in [
            "kb_claims",
            "kb_entities",
            "kb_relations",
            "kb_source_quality",
            "kb_contradictions",
        ] {
            assert!(names.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn ensure_schema_is_idempotent() {
        let db = test_db();
        ensure_knowledge_schema(&db).unwrap();
        ensure_knowledge_schema(&db).unwrap();
    }

    #[test]
    fn claim_polarity_round_trips_labels() {
        assert_eq!(ClaimPolarity::Positive.as_str(), "positive");
        assert_eq!(ClaimPolarity::from_str("negative"), ClaimPolarity::Negative);
        assert_eq!(ClaimPolarity::from_str("other"), ClaimPolarity::Positive);
    }
}
