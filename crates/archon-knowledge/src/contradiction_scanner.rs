use cozo::DbInstance;

use crate::errors::Result;
use crate::schema::{ClaimPolarity, ClaimRecord, ContradictionRecord};
use crate::{now_iso, stable_id, store};

pub fn scan_claims(claims: &[ClaimRecord]) -> Vec<ContradictionRecord> {
    let mut contradictions = Vec::new();
    for (left_idx, left) in claims.iter().enumerate() {
        for right in claims.iter().skip(left_idx + 1) {
            if contradicts(left, right) {
                contradictions.push(ContradictionRecord {
                    contradiction_id: stable_id(
                        "contradiction",
                        &[&left.claim_id, &right.claim_id],
                    ),
                    left_claim_id: left.claim_id.clone(),
                    right_claim_id: right.claim_id.clone(),
                    contradiction_type: "opposite_polarity_same_subject_predicate".into(),
                    explanation: format!("Claim '{}' conflicts with '{}'", left.text, right.text),
                    confidence: left.confidence.min(right.confidence),
                    created_at: now_iso(),
                });
            }
        }
    }
    contradictions
}

pub fn scan_and_store(db: &DbInstance) -> Result<Vec<ContradictionRecord>> {
    let claims = store::list_claims(db)?;
    let contradictions = scan_claims(&claims);
    for contradiction in &contradictions {
        store::insert_contradiction(db, contradiction)?;
    }
    Ok(contradictions)
}

fn contradicts(left: &ClaimRecord, right: &ClaimRecord) -> bool {
    left.normalized_subject == right.normalized_subject
        && left.normalized_predicate == right.normalized_predicate
        && matches!(
            (left.polarity, right.polarity),
            (ClaimPolarity::Positive, ClaimPolarity::Negative)
                | (ClaimPolarity::Negative, ClaimPolarity::Positive)
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(id: &str, polarity: ClaimPolarity) -> ClaimRecord {
        ClaimRecord {
            claim_id: id.into(),
            chunk_id: "chunk".into(),
            document_id: "doc".into(),
            text: id.into(),
            normalized_subject: "plugin".into(),
            normalized_predicate: "safe".into(),
            polarity,
            confidence: 0.9,
            created_at: "now".into(),
        }
    }

    #[test]
    fn detects_opposite_polarity_claims() {
        let contradictions = scan_claims(&[
            claim("left", ClaimPolarity::Positive),
            claim("right", ClaimPolarity::Negative),
        ]);
        assert_eq!(contradictions.len(), 1);
    }

    #[test]
    fn ignores_same_polarity_claims() {
        let contradictions = scan_claims(&[
            claim("left", ClaimPolarity::Positive),
            claim("right", ClaimPolarity::Positive),
        ]);
        assert!(contradictions.is_empty());
    }

    #[test]
    fn ignores_different_predicates() {
        let mut right = claim("right", ClaimPolarity::Negative);
        right.normalized_predicate = "fast".into();
        assert!(scan_claims(&[claim("left", ClaimPolarity::Positive), right]).is_empty());
    }
}
