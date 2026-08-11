//! Pre-action self-model predictions and their deterministic verification.
//!
//! `MetricEventKind::SelfModelPredictionEvaluated` was defined and threshold-ed
//! (`self_model_confidence_calibration_error`, see
//! [`crate::metrics::thresholds`]) but had no production emitter, so the gate it
//! feeds could never be evaluated. This is that emitter (issue #80).
//!
//! The ordering discipline is the one the shadow observer already establishes:
//!
//! * [`SelfModelPredictor::predict`] runs **before** the turn acts. It reads the
//!   self-model fact the turn's planning consumes and records the predicted
//!   probability, immutably, in `self_model_predictions`.
//! * [`SelfModelPredictor::resolve`] runs after finalisation. It attaches a
//!   deterministic verification to that row and emits the metric event carrying
//!   the *pre-action* probability. Nothing re-derives the prediction, so the
//!   comparison cannot become a rationalisation.
//!
//! A domain with no self-model fact produces **no prediction**. The metric
//! population is defined as self-model-backed turns; a default of 0.5 would put
//! turns the self-model knows nothing about into it and make the Brier score
//! describe the default instead of the model.

use std::collections::BTreeMap;

use archon_policy::CognitivePolicy;
use chrono::{DateTime, Utc};
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::CognitiveError;
use crate::cozo_guard::run_script_guarded;
use crate::metrics::definitions::{VERIFIED_FAILED, VERIFIED_PASSED};
use crate::metrics::emit::{MetricEmitter, runtime_cohort};
use crate::metrics::event::MetricEventKind;
use crate::schema::ensure_cognitive_schema;

/// Metric the emitted events derive into. Shared with the definition table so a
/// rename cannot leave events no definition reads.
pub const SELF_MODEL_CALIBRATION_METRIC: &str = "self_model_confidence_calibration_error";

/// Dimension label on both the fact and the prediction.
pub const TRUST_DIMENSION: &str = "domain_trust";

/// Where the deterministic label comes from. Named precisely: it is the
/// execution outcome of the turn's tool calls, not a judgement of task success.
pub const LABEL_SOURCE: &str = "live_turn_tool_execution";

/// Deterministic verdict on one finished turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnVerification {
    /// Every tool the turn invoked returned a non-error result and the turn
    /// finished.
    Passed,
    /// The turn did not finish, or at least one tool result was an error.
    Failed,
    /// Nothing deterministic to verify. Excluded from the calibration
    /// population rather than coerced to either side.
    Unknown,
}

impl TurnVerification {
    /// Outcome status written onto the metric event.
    ///
    /// `passed`/`failed` are the two statuses
    /// [`crate::metrics::derive`] admits into a Brier score; anything else is
    /// excluded there, so `unknown` cannot silently become a zero.
    pub fn outcome_status(self) -> &'static str {
        match self {
            Self::Passed => VERIFIED_PASSED,
            Self::Failed => VERIFIED_FAILED,
            Self::Unknown => "unknown",
        }
    }

    pub fn is_deterministic(self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

/// Deterministic evidence a finished turn produced.
///
/// Deliberately only counts: a tool result's `is_error` flag is a hard fact,
/// whereas "the user seemed satisfied" is not, and the roadmap forbids treating
/// unverified completion as success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnEvidence {
    pub tool_calls: u32,
    pub tool_failures: u32,
    pub completed: bool,
}

impl TurnEvidence {
    pub fn verdict(self) -> TurnVerification {
        if !self.completed || self.tool_failures > 0 {
            return TurnVerification::Failed;
        }
        if self.tool_calls == 0 {
            // A turn that executed nothing verified nothing. Calling that a
            // pass is exactly the "unverified completion is success" mistake
            // W5 exists to stop.
            return TurnVerification::Unknown;
        }
        TurnVerification::Passed
    }
}

/// One prediction, recorded before the action it predicts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelfModelPrediction {
    pub prediction_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub task_class: String,
    pub self_model_fact_id: String,
    pub self_model_dimension: String,
    pub predicted_success_probability: f32,
    pub fact_evidence_count: u64,
    pub created_at: DateTime<Utc>,
}

/// A prediction joined to its deterministic verification.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedSelfModelPrediction {
    pub prediction: SelfModelPrediction,
    pub verification_id: String,
    pub verification: TurnVerification,
    /// False when the verdict was not deterministic, so the row is resolved but
    /// contributes no calibration evidence.
    pub metric_recorded: bool,
}

pub struct SelfModelPredictor<'a> {
    db: &'a DbInstance,
    ledger_dir: std::path::PathBuf,
    policy: Option<CognitivePolicy>,
}

impl<'a> SelfModelPredictor<'a> {
    pub fn new(
        db: &'a DbInstance,
        ledger_dir: impl AsRef<std::path::Path>,
        policy: Option<CognitivePolicy>,
    ) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        Ok(Self {
            db,
            ledger_dir: ledger_dir.as_ref().to_path_buf(),
            policy,
        })
    }

    /// Record what the self-model expects of this turn, before the turn acts.
    ///
    /// `Ok(None)` when the domain has no self-model fact: absence of evidence is
    /// recorded as absence, never as a neutral prediction.
    pub fn predict(
        &self,
        session_id: &str,
        turn_number: u64,
        domain: &str,
    ) -> Result<Option<SelfModelPrediction>, CognitiveError> {
        let fact_id = format!("{TRUST_DIMENSION}:{domain}");
        let Some((confidence, evidence_count)) = self.trust_fact(&fact_id)? else {
            return Ok(None);
        };
        let prediction = SelfModelPrediction {
            // Derived, not random: a retried turn start must reach the same id
            // so the pending row is replaced rather than duplicated.
            prediction_id: format!("smp:{session_id}:{turn_number}:{fact_id}"),
            session_id: session_id.to_owned(),
            turn_number,
            task_class: domain.to_owned(),
            self_model_fact_id: fact_id,
            self_model_dimension: TRUST_DIMENSION.to_owned(),
            predicted_success_probability: confidence.clamp(0.0, 1.0),
            fact_evidence_count: evidence_count,
            created_at: Utc::now(),
        };
        self.put_pending(&prediction)?;
        Ok(Some(prediction))
    }

    /// Attach the deterministic verification and emit the calibration event.
    ///
    /// `Ok(None)` when the turn made no prediction (no self-model fact, or the
    /// pre-action step never ran).
    pub fn resolve(
        &self,
        session_id: &str,
        turn_number: u64,
        evidence: TurnEvidence,
        model_id: &str,
    ) -> Result<Option<ResolvedSelfModelPrediction>, CognitiveError> {
        let Some(prediction) = self.take_pending(session_id, turn_number)? else {
            return Ok(None);
        };
        let verification = evidence.verdict();
        let verification_id = format!("turn_exec:{session_id}:{turn_number}");
        self.mark_resolved(&prediction, &verification_id, verification)?;
        let metric_recorded = if verification.is_deterministic() {
            self.record_calibration(&prediction, &verification_id, verification, model_id)?
        } else {
            false
        };
        Ok(Some(ResolvedSelfModelPrediction {
            prediction,
            verification_id,
            verification,
            metric_recorded,
        }))
    }

    /// Confidence and evidence count of a domain-trust fact, if one exists.
    pub fn trust_fact(&self, fact_id: &str) -> Result<Option<(f32, u64)>, CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert("fact_id".into(), DataValue::from(fact_id));
        let rows = run_script_guarded(
            self.db,
            "?[confidence, evidence_count] := *self_model_facts{fact_id: $fact_id, confidence, evidence_count}",
            params,
            ScriptMutability::Immutable,
            "read self-model trust fact for prediction",
        )?;
        Ok(rows.rows.first().and_then(|row| {
            let confidence = row[0].get_float()? as f32;
            confidence
                .is_finite()
                .then(|| (confidence, row[1].get_int().unwrap_or(0).max(0) as u64))
        }))
    }

    fn put_pending(&self, prediction: &SelfModelPrediction) -> Result<(), CognitiveError> {
        let mut params = prediction_params(prediction);
        params.insert("resolved".into(), DataValue::from(false));
        params.insert("verification_id".into(), DataValue::from(""));
        params.insert("verified_outcome".into(), DataValue::from(""));
        params.insert("resolved_at".into(), DataValue::from(""));
        run_script_guarded(
            self.db,
            &put_script(),
            params,
            ScriptMutability::Mutable,
            "put pending self-model prediction",
        )?;
        Ok(())
    }

    fn take_pending(
        &self,
        session_id: &str,
        turn_number: u64,
    ) -> Result<Option<SelfModelPrediction>, CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert("session_id".into(), DataValue::from(session_id));
        params.insert("turn_number".into(), DataValue::from(turn_number as i64));
        let rows = run_script_guarded(
            self.db,
            "?[prediction_id, task_class, self_model_fact_id, self_model_dimension, predicted_success_probability, fact_evidence_count, created_at] := \
             *self_model_predictions{prediction_id, session_id: $session_id, turn_number: $turn_number, task_class, self_model_fact_id, self_model_dimension, predicted_success_probability, fact_evidence_count, resolved: false, created_at}",
            params,
            ScriptMutability::Immutable,
            "read pending self-model prediction",
        )?;
        let mut predictions: Vec<SelfModelPrediction> = rows
            .rows
            .iter()
            .map(|row| SelfModelPrediction {
                prediction_id: str_col(row, 0),
                session_id: session_id.to_owned(),
                turn_number,
                task_class: str_col(row, 1),
                self_model_fact_id: str_col(row, 2),
                self_model_dimension: str_col(row, 3),
                predicted_success_probability: row[4].get_float().unwrap_or(0.0) as f32,
                fact_evidence_count: row[5].get_int().unwrap_or(0).max(0) as u64,
                created_at: parse_time(&str_col(row, 6)),
            })
            .collect();
        predictions.sort_by(|left, right| left.prediction_id.cmp(&right.prediction_id));
        Ok(predictions.pop())
    }

    /// Rewrite only the verification columns.
    ///
    /// `predicted_success_probability` and `created_at` are carried over from
    /// the stored row rather than recomputed, so resolution cannot move the
    /// prediction it is grading.
    fn mark_resolved(
        &self,
        prediction: &SelfModelPrediction,
        verification_id: &str,
        verification: TurnVerification,
    ) -> Result<(), CognitiveError> {
        let mut params = prediction_params(prediction);
        params.insert("resolved".into(), DataValue::from(true));
        params.insert("verification_id".into(), DataValue::from(verification_id));
        params.insert(
            "verified_outcome".into(),
            DataValue::from(verification.outcome_status()),
        );
        params.insert(
            "resolved_at".into(),
            DataValue::from(Utc::now().to_rfc3339().as_str()),
        );
        run_script_guarded(
            self.db,
            &put_script(),
            params,
            ScriptMutability::Mutable,
            "resolve self-model prediction",
        )?;
        Ok(())
    }

    fn record_calibration(
        &self,
        prediction: &SelfModelPrediction,
        verification_id: &str,
        verification: TurnVerification,
        model_id: &str,
    ) -> Result<bool, CognitiveError> {
        let emitter = MetricEmitter::open(
            self.db,
            &self.ledger_dir,
            runtime_cohort(&prediction.task_class, model_id, self.policy.as_ref()),
        )?;
        let mut event = emitter
            .event(
                SELF_MODEL_CALIBRATION_METRIC,
                MetricEventKind::SelfModelPredictionEvaluated,
                &prediction.prediction_id,
                Utc::now(),
            )
            .with_session(&prediction.session_id, prediction.turn_number)
            .with_value(f64::from(prediction.predicted_success_probability))
            .with_outcome(verification.outcome_status())
            .with_identity("self_model_prediction_id", &prediction.prediction_id)
            .with_identity("self_model_fact_id", &prediction.self_model_fact_id)
            .with_identity("self_model_dimension", &prediction.self_model_dimension)
            .with_identity("self_model_backed", "true")
            .with_identity("verification_id", verification_id)
            .with_identity(
                "self_model_evidence_count",
                prediction.fact_evidence_count.to_string(),
            );
        event.label_source = LABEL_SOURCE.into();
        event.evidence_refs = vec![
            format!("self_model_fact:{}", prediction.self_model_fact_id),
            format!("self_model_prediction:{}", prediction.prediction_id),
            format!("verification:{verification_id}"),
        ];
        Ok(matches!(
            emitter.record(&event)?,
            crate::metrics::MetricWriteOutcome::Written
        ))
    }
}

const COLUMNS: &str = "prediction_id, session_id, turn_number, task_class, self_model_fact_id, self_model_dimension, predicted_success_probability, fact_evidence_count, resolved, verification_id, verified_outcome, created_at, resolved_at";

fn put_script() -> String {
    format!(
        "?[{COLUMNS}] <- [[$prediction_id, $session_id, $turn_number, $task_class, \
         $self_model_fact_id, $self_model_dimension, $predicted_success_probability, \
         $fact_evidence_count, $resolved, $verification_id, $verified_outcome, $created_at, \
         $resolved_at]]
         :put self_model_predictions {{ prediction_id => session_id, turn_number, task_class, \
         self_model_fact_id, self_model_dimension, predicted_success_probability, \
         fact_evidence_count, resolved, verification_id, verified_outcome, created_at, resolved_at }}"
    )
}

fn prediction_params(prediction: &SelfModelPrediction) -> BTreeMap<String, DataValue> {
    let mut params = BTreeMap::new();
    params.insert(
        "prediction_id".into(),
        DataValue::from(prediction.prediction_id.as_str()),
    );
    params.insert(
        "session_id".into(),
        DataValue::from(prediction.session_id.as_str()),
    );
    params.insert(
        "turn_number".into(),
        DataValue::from(prediction.turn_number as i64),
    );
    params.insert(
        "task_class".into(),
        DataValue::from(prediction.task_class.as_str()),
    );
    params.insert(
        "self_model_fact_id".into(),
        DataValue::from(prediction.self_model_fact_id.as_str()),
    );
    params.insert(
        "self_model_dimension".into(),
        DataValue::from(prediction.self_model_dimension.as_str()),
    );
    params.insert(
        "predicted_success_probability".into(),
        DataValue::from(f64::from(prediction.predicted_success_probability)),
    );
    params.insert(
        "fact_evidence_count".into(),
        DataValue::from(prediction.fact_evidence_count as i64),
    );
    params.insert(
        "created_at".into(),
        DataValue::from(prediction.created_at.to_rfc3339().as_str()),
    );
    params
}

fn str_col(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

fn parse_time(value: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}
