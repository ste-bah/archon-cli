use cozo::DbInstance;

use crate::errors::Result;
use crate::schema::SourceQualityRecord;
use crate::{now_iso, store};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityOutcome {
    CitationVerified,
    UserAccepted,
    UserCorrected,
    ContradictionFound,
}

impl QualityOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CitationVerified => "CitationVerified",
            Self::UserAccepted => "UserAccepted",
            Self::UserCorrected => "UserCorrected",
            Self::ContradictionFound => "ContradictionFound",
        }
    }

    fn delta(self) -> f64 {
        match self {
            Self::CitationVerified => 0.08,
            Self::UserAccepted => 0.05,
            Self::UserCorrected => -0.12,
            Self::ContradictionFound => -0.18,
        }
    }
}

pub fn seed_source_quality(db: &DbInstance, source_id: &str) -> Result<SourceQualityRecord> {
    if let Some(existing) = store::get_source_quality(db, source_id)? {
        return Ok(existing);
    }
    let record = SourceQualityRecord {
        source_id: source_id.to_string(),
        score: 0.5,
        observations: 0,
        last_outcome: "Seeded".into(),
        updated_at: now_iso(),
    };
    store::upsert_source_quality(db, &record)?;
    Ok(record)
}

pub fn update_from_outcome(
    db: &DbInstance,
    source_id: &str,
    outcome: QualityOutcome,
) -> Result<SourceQualityRecord> {
    let current = seed_source_quality(db, source_id)?;
    let next = SourceQualityRecord {
        source_id: source_id.to_string(),
        score: (current.score + outcome.delta()).clamp(0.0, 1.0),
        observations: current.observations + 1,
        last_outcome: outcome.as_str().into(),
        updated_at: now_iso(),
    };
    store::upsert_source_quality(db, &next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ensure_knowledge_schema;

    fn test_db() -> DbInstance {
        DbInstance::new("mem", "", "").unwrap()
    }

    #[test]
    fn seeds_source_at_neutral_score() {
        let db = test_db();
        ensure_knowledge_schema(&db).unwrap();
        let record = seed_source_quality(&db, "doc-1").unwrap();
        assert_eq!(record.score, 0.5);
        assert_eq!(record.observations, 0);
    }

    #[test]
    fn positive_outcome_increases_score() {
        let db = test_db();
        ensure_knowledge_schema(&db).unwrap();
        let record = update_from_outcome(&db, "doc-1", QualityOutcome::CitationVerified).unwrap();
        assert!(record.score > 0.5);
        assert_eq!(record.observations, 1);
    }

    #[test]
    fn negative_outcome_decreases_score() {
        let db = test_db();
        ensure_knowledge_schema(&db).unwrap();
        let record = update_from_outcome(&db, "doc-1", QualityOutcome::ContradictionFound).unwrap();
        assert!(record.score < 0.5);
    }
}
