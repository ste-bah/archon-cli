//! Raw cognitive metric events: the append-only source of truth for R8.
//!
//! Derived metrics are recomputed from these rows, never from mutable
//! counters, so a metric definition can change without rewriting history.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::CognitiveError;
use crate::metrics::window::MetricCohort;

/// Version of the raw event schema. Bumping this is a code change, not a
/// row edit, so prior evaluation windows stay recomputable.
pub const METRIC_EVENT_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricEventKind {
    CorrectionClassified,
    ShadowDecisionCompared,
    AttributionEvaluated,
    SelfModelPredictionEvaluated,
    SelfModelFactUpdated,
    RetrievalHitObserved,
    RuleLifecycleObserved,
    GovernedProposalObserved,
    PromptRulesComposed,
    WorldLabelMaterialized,
    SurpriseObserved,
}

impl MetricEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CorrectionClassified => "correction_classified",
            Self::ShadowDecisionCompared => "shadow_decision_compared",
            Self::AttributionEvaluated => "attribution_evaluated",
            Self::SelfModelPredictionEvaluated => "self_model_prediction_evaluated",
            Self::SelfModelFactUpdated => "self_model_fact_updated",
            Self::RetrievalHitObserved => "retrieval_hit_observed",
            Self::RuleLifecycleObserved => "rule_lifecycle_observed",
            Self::GovernedProposalObserved => "governed_proposal_observed",
            Self::PromptRulesComposed => "prompt_rules_composed",
            Self::WorldLabelMaterialized => "world_label_materialized",
            Self::SurpriseObserved => "surprise_observed",
        }
    }

    /// Identity/provenance keys the roadmap declares mandatory for this kind.
    ///
    /// A metric that cannot be traced back to the decision, action, or
    /// adjudication it measures is not evidence, so these are rejected at
    /// write time rather than discovered as holes during derivation.
    pub fn required_identities(self) -> &'static [&'static str] {
        match self {
            Self::CorrectionClassified => &[
                "correction_id",
                "predicted_label",
                "ground_truth_label",
                "abstained",
            ],
            Self::ShadowDecisionCompared => &[
                "shadow_decision_id",
                "decision_id",
                "live_action_id",
                "candidate_id",
                "candidate_rank",
            ],
            Self::AttributionEvaluated => &[
                "correction_id",
                "decision_id",
                "action_attempt_id",
                "causal_candidate_id",
                "adjudicated_causal_candidate_id",
                "cause_action_class",
                "attribution_adjudication_id",
                "accepted",
                "abstained",
            ],
            Self::SelfModelPredictionEvaluated => &[
                "self_model_prediction_id",
                "self_model_fact_id",
                "self_model_dimension",
                "self_model_backed",
                "verification_id",
            ],
            Self::SelfModelFactUpdated => &[
                "self_model_fact_id",
                "self_model_dimension",
                "self_model_version",
            ],
            Self::RetrievalHitObserved => &["retrieval_hit_id", "lesson_id", "rule_injected"],
            Self::RuleLifecycleObserved => &["rule_id", "rule_operation"],
            Self::GovernedProposalObserved => &[
                "governed_proposal_id",
                "proposal_kind",
                "proposal_lifecycle_operation",
            ],
            Self::PromptRulesComposed => &[
                "prompt_snapshot_id",
                "rule_state_snapshot_id",
                "ordered_injected_rule_ids",
                "stale_definition_version",
            ],
            Self::WorldLabelMaterialized => &[
                "action_attempt_id",
                "prediction_id",
                "verification_id",
                "label_definition_version",
            ],
            Self::SurpriseObserved => &["prediction_id", "action_attempt_id", "verification_id"],
        }
    }

    /// Kinds whose `value` is a probability and is therefore range-checked.
    fn value_is_probability(self) -> bool {
        matches!(
            self,
            Self::SelfModelPredictionEvaluated | Self::SelfModelFactUpdated
        )
    }
}

impl std::str::FromStr for MetricEventKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "correction_classified" => Self::CorrectionClassified,
            "shadow_decision_compared" => Self::ShadowDecisionCompared,
            "attribution_evaluated" => Self::AttributionEvaluated,
            "self_model_prediction_evaluated" => Self::SelfModelPredictionEvaluated,
            "self_model_fact_updated" => Self::SelfModelFactUpdated,
            "retrieval_hit_observed" => Self::RetrievalHitObserved,
            "rule_lifecycle_observed" => Self::RuleLifecycleObserved,
            "governed_proposal_observed" => Self::GovernedProposalObserved,
            "prompt_rules_composed" => Self::PromptRulesComposed,
            "world_label_materialized" => Self::WorldLabelMaterialized,
            "surprise_observed" => Self::SurpriseObserved,
            _ => return Err(()),
        })
    }
}

/// One immutable measurement observation.
///
/// The event-kind-specific identity columns from the roadmap schema live in
/// [`CognitiveMetricEvent::identities`] instead of ~70 mostly-null relation
/// columns; [`MetricEventKind::required_identities`] is what makes that map
/// enforceable rather than free-form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveMetricEvent {
    pub metric_event_id: String,
    pub idempotency_key: String,
    pub metric_name: String,
    pub metric_definition_version: i64,
    pub evaluation_dataset_version: String,
    pub evaluation_window_id: String,
    pub event_kind: MetricEventKind,
    pub session_id: String,
    pub turn_number: u64,
    pub cohort: MetricCohort,
    pub label_source: String,
    pub outcome_status: String,
    pub value: Option<f64>,
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    pub identities: BTreeMap<String, String>,
    pub evidence_refs: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl CognitiveMetricEvent {
    /// Build an event with only the always-required fields populated.
    ///
    /// Callers layer identities and numerics on top; [`Self::validate`] is
    /// still the single place that decides whether the result is writable.
    pub fn new(
        metric_event_id: impl Into<String>,
        metric_name: impl Into<String>,
        event_kind: MetricEventKind,
        evaluation_window_id: impl Into<String>,
        cohort: MetricCohort,
        created_at: DateTime<Utc>,
    ) -> Self {
        let metric_event_id = metric_event_id.into();
        Self {
            idempotency_key: metric_event_id.clone(),
            metric_event_id,
            metric_name: metric_name.into(),
            metric_definition_version: METRIC_EVENT_SCHEMA_VERSION,
            evaluation_dataset_version: "v1".into(),
            evaluation_window_id: evaluation_window_id.into(),
            event_kind,
            session_id: String::new(),
            turn_number: 0,
            cohort,
            label_source: String::new(),
            outcome_status: String::new(),
            value: None,
            numerator: None,
            denominator: None,
            identities: BTreeMap::new(),
            evidence_refs: Vec::new(),
            created_at,
        }
    }

    pub fn with_identity(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.identities.insert(key.into(), value.into());
        self
    }

    pub fn with_session(mut self, session_id: impl Into<String>, turn_number: u64) -> Self {
        self.session_id = session_id.into();
        self.turn_number = turn_number;
        self
    }

    pub fn with_value(mut self, value: f64) -> Self {
        self.value = Some(value);
        self
    }

    pub fn with_ratio(mut self, numerator: f64, denominator: f64) -> Self {
        self.numerator = Some(numerator);
        self.denominator = Some(denominator);
        self
    }

    pub fn with_outcome(mut self, outcome_status: impl Into<String>) -> Self {
        self.outcome_status = outcome_status.into();
        self
    }

    pub fn identity(&self, key: &str) -> Option<&str> {
        self.identities.get(key).map(String::as_str)
    }

    /// Content hash used to tell an idempotent replay apart from a conflict.
    pub fn fingerprint(&self) -> Result<String, CognitiveError> {
        let canonical = serde_json::to_vec(self)?;
        Ok(blake3::hash(&canonical).to_hex().to_string())
    }

    pub fn validate(&self) -> Result<(), CognitiveError> {
        require_text("metric_event_id", &self.metric_event_id)?;
        require_text("idempotency_key", &self.idempotency_key)?;
        require_text("metric_name", &self.metric_name)?;
        require_text("evaluation_window_id", &self.evaluation_window_id)?;
        require_text(
            "evaluation_dataset_version",
            &self.evaluation_dataset_version,
        )?;
        require_text("task_class", &self.cohort.task_class)?;
        require_text("model_id", &self.cohort.model_id)?;
        require_text("policy_version", &self.cohort.policy_version)?;
        if self.metric_definition_version <= 0 {
            return Err(invalid("metric_definition_version must be positive"));
        }

        require_finite("value", self.value)?;
        require_finite("numerator", self.numerator)?;
        require_finite("denominator", self.denominator)?;
        if self
            .denominator
            .is_some_and(|denominator| denominator < 0.0)
        {
            return Err(invalid("denominator must not be negative"));
        }
        if self.event_kind.value_is_probability()
            && self
                .value
                .is_some_and(|value| !(0.0..=1.0).contains(&value))
        {
            return Err(invalid(&format!(
                "{} requires value in [0,1]",
                self.event_kind.as_str()
            )));
        }

        for key in self.event_kind.required_identities() {
            match self.identities.get(*key) {
                Some(value) if !value.trim().is_empty() => {}
                _ => {
                    return Err(invalid(&format!(
                        "{} requires identity `{key}`",
                        self.event_kind.as_str()
                    )));
                }
            }
        }
        Ok(())
    }
}

fn require_text(field: &str, value: &str) -> Result<(), CognitiveError> {
    if value.trim().is_empty() {
        return Err(invalid(&format!("{field} must not be empty")));
    }
    Ok(())
}

/// NaN and infinity would poison every downstream sum, mean, and percentile,
/// and Cozo cannot store them faithfully either. Reject at the boundary.
fn require_finite(field: &str, value: Option<f64>) -> Result<(), CognitiveError> {
    match value {
        Some(value) if !value.is_finite() => {
            Err(invalid(&format!("{field} must be finite, got {value}")))
        }
        _ => Ok(()),
    }
}

fn invalid(message: &str) -> CognitiveError {
    CognitiveError::Metric(format!("invalid metric event: {message}"))
}
