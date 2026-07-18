use std::collections::BTreeMap;

use crate::models::{
    RequiredEvidence, RequiredEvidenceCheck, RequiredEvidenceKind, RequiredEvidenceStatus,
};

pub fn check_required_evidence(
    required: &[RequiredEvidenceKind],
    evidence: &[RequiredEvidence],
) -> RequiredEvidenceCheck {
    let mut latest = BTreeMap::new();
    for item in evidence {
        let replace = latest
            .get(&item.kind)
            .is_none_or(|current: &&RequiredEvidence| current.sequence <= item.sequence);
        if replace {
            latest.insert(item.kind, item);
        }
    }

    let mut missing = Vec::new();
    let mut failed = Vec::new();
    for kind in required {
        match latest.get(kind).map(|item| item.status) {
            Some(RequiredEvidenceStatus::Passed) => {}
            Some(RequiredEvidenceStatus::Failed) => failed.push(*kind),
            _ => missing.push(*kind),
        }
    }
    RequiredEvidenceCheck {
        allowed: missing.is_empty() && failed.is_empty(),
        missing,
        failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_requirements_block_without_claims_or_evidence() {
        let result = check_required_evidence(
            &[RequiredEvidenceKind::Tests, RequiredEvidenceKind::Build],
            &[],
        );

        assert!(!result.allowed);
        assert_eq!(
            result.missing,
            vec![RequiredEvidenceKind::Tests, RequiredEvidenceKind::Build]
        );
    }

    #[test]
    fn explicit_requirements_use_latest_matching_evidence() {
        let result = check_required_evidence(
            &[RequiredEvidenceKind::Tests],
            &[
                RequiredEvidence {
                    kind: RequiredEvidenceKind::Tests,
                    status: RequiredEvidenceStatus::Failed,
                    sequence: 1,
                },
                RequiredEvidence {
                    kind: RequiredEvidenceKind::Tests,
                    status: RequiredEvidenceStatus::Passed,
                    sequence: 2,
                },
            ],
        );

        assert!(result.allowed);
        assert!(result.missing.is_empty());
        assert!(result.failed.is_empty());
    }
}
