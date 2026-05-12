use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

use crate::types::{ConfidenceSignal, ReasoningEventKind};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeverityOverride {
    pub value: f32,
    pub reason: String,
}

impl SeverityOverride {
    pub fn validate(&self) -> Result<()> {
        if !(0.0..=1.0).contains(&self.value) {
            return Err(anyhow!("severity override must be within [0.0, 1.0]"));
        }
        if self.reason.trim().is_empty() {
            return Err(anyhow!("severity override requires a persisted reason"));
        }
        Ok(())
    }
}

pub fn base_severity(kind: ReasoningEventKind) -> f32 {
    match kind {
        ReasoningEventKind::ClaimCorrectedByUser => 1.0,
        ReasoningEventKind::ClaimContradictedBySource => 1.0,
        ReasoningEventKind::RepeatedReasoningFailure => 0.9,
        ReasoningEventKind::ShadowRepeatedPattern => 0.6,
        ReasoningEventKind::ClaimBeforeSourceRead => 0.7,
        ReasoningEventKind::UnsupportedClaim => 0.6,
        ReasoningEventKind::CompletionClaimWithoutEvidence => 0.6,
        ReasoningEventKind::TestStatusClaimWithoutCommand => 0.6,
        ReasoningEventKind::PlanClaimDrift => 0.6,
        ReasoningEventKind::VerificationNeeded => 0.4,
        ReasoningEventKind::CriticUnavailable => 0.2,
        ReasoningEventKind::UncertaintyDisclosed => 0.1,
        ReasoningEventKind::BriefingUpdatedWithTaskContext => 0.0,
        ReasoningEventKind::ClaimEmitted => 0.0,
        ReasoningEventKind::SourceVerifiedClaim => 0.0,
    }
}

pub fn confidence_modifier(confidence: ConfidenceSignal) -> f32 {
    match confidence {
        ConfidenceSignal::Confident => 1.0,
        ConfidenceSignal::Qualified => 0.8,
        ConfidenceSignal::Uncertain => 0.5,
        ConfidenceSignal::ExplicitlySpeculative => 0.2,
        ConfidenceSignal::Unknown => 0.7,
    }
}

pub fn effective_severity(
    kind: ReasoningEventKind,
    confidence: ConfidenceSignal,
    override_: Option<&SeverityOverride>,
) -> Result<f32> {
    if let Some(override_) = override_ {
        override_.validate()?;
        return Ok(override_.value);
    }

    let mut value = base_severity(kind) * confidence_modifier(confidence);
    if matches!(
        kind,
        ReasoningEventKind::ClaimCorrectedByUser | ReasoningEventKind::ClaimContradictedBySource
    ) {
        value = value.max(0.7);
    }
    Ok(value.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_modifier_softens_uncertain_claims() {
        let confident = effective_severity(
            ReasoningEventKind::UnsupportedClaim,
            ConfidenceSignal::Confident,
            None,
        )
        .unwrap();
        let uncertain = effective_severity(
            ReasoningEventKind::UnsupportedClaim,
            ConfidenceSignal::Uncertain,
            None,
        )
        .unwrap();
        assert!(confident > uncertain);
    }

    #[test]
    fn contradiction_has_minimum_severity() {
        let value = effective_severity(
            ReasoningEventKind::ClaimContradictedBySource,
            ConfidenceSignal::ExplicitlySpeculative,
            None,
        )
        .unwrap();
        assert_eq!(value, 0.7);
    }
}
