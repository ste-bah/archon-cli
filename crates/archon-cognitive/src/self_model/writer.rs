//! Production writer for the self-model.
//!
//! `SelfModelStore::write_fact` existed and was tested, but nothing outside
//! tests ever called it, so `archon cognitive status` reported `Self-model
//! facts: 0` forever. This is the caller (issue #80), and it is deliberately
//! conservative:
//!
//! * facts come from verified aggregate statistics over recorded reflections,
//!   never from a single turn and never from an unverified claim;
//! * confidence moves by at most [`MAX_CONFIDENCE_DRIFT`] per refresh, so one
//!   bad stretch cannot swing the model to an extreme it will act on;
//! * a domain with too little evidence produces *no fact at all* and says so,
//!   rather than a placeholder at 0.5 that a reader could not tell apart from a
//!   measured neutral result.

use std::collections::BTreeMap;

use archon_policy::CognitivePolicy;
use chrono::Utc;
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::cozo_guard::run_script_guarded;
use crate::executive_support::domain_for;
use crate::metrics::emit::{MetricEmitter, runtime_cohort};
use crate::metrics::event::MetricEventKind;
// The dimension label is shared with the pre-action predictor: it reads the
// facts this writer produces, and two independent copies of the label would let
// the reader silently miss them.
use crate::self_model::prediction::TRUST_DIMENSION;
use crate::self_model::store::SelfModelStore;
use crate::self_model::types::{FactKind, SelfModelFact};
use crate::{CognitiveError, OutcomeSummary, SituationKind};

/// Verified observations a domain needs before it gets a fact.
///
/// Three, matching the confidence floor the reader already applies: below it
/// `get_domain_trust` clamps to `[0.4, 0.6]` anyway, so writing a fact would
/// add a row without adding information.
pub const MIN_EVIDENCE_FOR_FACT: u64 = 3;

/// Largest confidence movement one refresh may apply.
///
/// The self-model feeds candidate planning. A fact that could jump from 0.9 to
/// 0.1 in one tick would make the planner oscillate on noise; bounded drift
/// makes it take sustained evidence to move.
pub const MAX_CONFIDENCE_DRIFT: f32 = 0.05;

/// Confidence assumed for a domain that has no fact yet.
const NEUTRAL_CONFIDENCE: f32 = 0.5;

const METRIC_NAME: &str = "self_model_fact_confidence_mean";

/// What one refresh did, including what it declined to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SelfModelUpdate {
    pub facts_written: usize,
    pub metrics_emitted: usize,
    /// Domains that produced no fact, each with why. Visible so "no facts" is
    /// never mistaken for "the writer did not run".
    pub unwritten: Vec<String>,
    pub errors: Vec<String>,
}

impl SelfModelUpdate {
    /// `None` when the writer was not allowed to run at all, which is a
    /// different claim from `Some(false)` ("it ran and changed nothing").
    pub fn changed(&self) -> bool {
        self.facts_written > 0
    }
}

/// One domain's verified tally.
#[derive(Debug, Clone, Copy, Default)]
struct DomainTally {
    successes: u64,
    verified: u64,
}

pub struct SelfModelWriter<'a> {
    db: &'a DbInstance,
    ledger_dir: std::path::PathBuf,
    policy: Option<CognitivePolicy>,
}

impl<'a> SelfModelWriter<'a> {
    pub fn new(
        db: &'a DbInstance,
        ledger_dir: impl AsRef<std::path::Path>,
        policy: Option<CognitivePolicy>,
    ) -> Self {
        Self {
            db,
            ledger_dir: ledger_dir.as_ref().to_path_buf(),
            policy,
        }
    }

    /// Recompute domain-trust facts from recorded reflections.
    ///
    /// `Ok(None)` when policy withholds self-model updates: the caller must be
    /// able to report "not permitted" rather than "nothing to update".
    pub fn refresh_domain_trust(&self) -> Result<Option<SelfModelUpdate>, CognitiveError> {
        if !self
            .policy
            .as_ref()
            .is_some_and(|policy| policy.allow_self_model_updates)
        {
            return Ok(None);
        }
        let tallies = self.domain_tallies()?;
        let store = SelfModelStore::new(self.db)?;
        let mut update = SelfModelUpdate::default();

        for (domain, tally) in tallies {
            if tally.verified < MIN_EVIDENCE_FOR_FACT {
                update.unwritten.push(format!(
                    "insufficient_evidence:{domain}:{}/{MIN_EVIDENCE_FOR_FACT}",
                    tally.verified
                ));
                continue;
            }
            let target = tally.successes as f32 / tally.verified as f32;
            let existing = self.existing_fact(&domain)?;
            let previous = existing
                .map(|(confidence, _)| confidence)
                .unwrap_or(NEUTRAL_CONFIDENCE);
            let confidence = drift(previous, target);
            // Nothing new to say: rewriting the identical fact would produce a
            // fresh `last_seen_at` implying evidence that did not arrive.
            if existing.map(|(_, count)| count) == Some(tally.verified) && confidence == previous {
                update
                    .unwritten
                    .push(format!("unchanged:{domain}:{}", tally.verified));
                continue;
            }

            let fact = trust_fact(&domain, confidence, tally.verified);
            store.write_fact(&fact)?;
            update.facts_written += 1;
            match self.emit_fact_metric(&fact) {
                Ok(true) => update.metrics_emitted += 1,
                Ok(false) => {}
                Err(error) => update.errors.push(format!("self_model_metric:{error}")),
            }
        }
        Ok(Some(update))
    }

    /// Verified outcomes per domain.
    ///
    /// `PartialSuccess` and `Degraded` are excluded rather than scored: they
    /// are explicitly not deterministic verdicts, and folding them in either
    /// direction would be inventing a label.
    fn domain_tallies(&self) -> Result<BTreeMap<String, DomainTally>, CognitiveError> {
        // `reflection_id` stays in the head: a projection that dropped it would
        // be deduplicated by Cozo, collapsing every reflection sharing a
        // (kind, outcome) pair into one and silently under-counting evidence.
        let rows = run_script_guarded(
            self.db,
            "?[reflection_id, situation_kind, outcome] := *cognitive_reflections{reflection_id, situation_kind, outcome}",
            Default::default(),
            ScriptMutability::Immutable,
            "tally cognitive reflections by domain",
        )?;
        let mut tallies: BTreeMap<String, DomainTally> = BTreeMap::new();
        for row in &rows.rows {
            let kind = situation_kind(row[1].get_str().unwrap_or(""));
            let Some(success) = verified_outcome(row[2].get_str().unwrap_or("")) else {
                continue;
            };
            let entry = tallies.entry(domain_for(kind).to_string()).or_default();
            entry.verified += 1;
            entry.successes += u64::from(success);
        }
        Ok(tallies)
    }

    /// Current confidence and evidence count for a domain's trust fact.
    fn existing_fact(&self, domain: &str) -> Result<Option<(f32, u64)>, CognitiveError> {
        let mut params = BTreeMap::new();
        params.insert(
            "fact_id".into(),
            DataValue::from(trust_fact_id(domain).as_str()),
        );
        let rows = run_script_guarded(
            self.db,
            "?[confidence, evidence_count] := *self_model_facts{fact_id: $fact_id, confidence, evidence_count}",
            params,
            ScriptMutability::Immutable,
            "read existing self-model trust fact",
        )?;
        Ok(rows.rows.first().map(|row| {
            (
                row[0].get_float().unwrap_or(f64::from(NEUTRAL_CONFIDENCE)) as f32,
                row[1].get_int().unwrap_or(0).max(0) as u64,
            )
        }))
    }

    fn emit_fact_metric(&self, fact: &SelfModelFact) -> Result<bool, CognitiveError> {
        let emitter = MetricEmitter::open(
            self.db,
            &self.ledger_dir,
            runtime_cohort(&fact.domain, TRUST_DIMENSION, self.policy.as_ref()),
        )?;
        let version = fact.evidence_count;
        let subject = format!("{}:{version}:{}", fact.id, permille(fact.confidence));
        let mut event = emitter
            .event(
                METRIC_NAME,
                MetricEventKind::SelfModelFactUpdated,
                &subject,
                fact.last_seen_at,
            )
            .with_value(f64::from(fact.confidence))
            .with_outcome("written")
            .with_identity("self_model_fact_id", &fact.id)
            .with_identity("self_model_dimension", TRUST_DIMENSION)
            .with_identity("self_model_version", version.to_string());
        event.label_source = "verified_reflection_aggregate".into();
        event.evidence_refs = vec![format!("self_model_fact:{}", fact.id)];
        Ok(matches!(
            emitter.record(&event)?,
            crate::metrics::MetricWriteOutcome::Written
        ))
    }
}

fn trust_fact_id(domain: &str) -> String {
    format!("domain_trust:{domain}")
}

/// Stable id per domain so a refresh updates the fact instead of appending a
/// rival copy of it.
fn trust_fact(domain: &str, confidence: f32, evidence_count: u64) -> SelfModelFact {
    let now = Utc::now();
    SelfModelFact {
        id: trust_fact_id(domain),
        domain: domain.to_string(),
        fact_kind: FactKind::DomainTrust,
        statement: format!("verified success rate over {evidence_count} reflected outcomes"),
        confidence: confidence.clamp(0.0, 1.0),
        evidence_count,
        last_seen_at: now,
        expires_at: None,
        created_at: now,
    }
}

/// Move `previous` toward `target` by at most [`MAX_CONFIDENCE_DRIFT`].
fn drift(previous: f32, target: f32) -> f32 {
    let delta = (target - previous).clamp(-MAX_CONFIDENCE_DRIFT, MAX_CONFIDENCE_DRIFT);
    (previous + delta).clamp(0.0, 1.0)
}

fn permille(value: f32) -> i64 {
    (f64::from(value) * 1000.0).round() as i64
}

/// `Some(true)` for a verified pass, `Some(false)` for a verified failure,
/// `None` for an outcome that is not a deterministic verdict.
fn verified_outcome(value: &str) -> Option<bool> {
    match value {
        _ if value == OutcomeSummary::Success.as_str() => Some(true),
        _ if value == OutcomeSummary::Failure.as_str() => Some(false),
        _ if value == OutcomeSummary::UserCorrected.as_str() => Some(false),
        _ => None,
    }
}

fn situation_kind(value: &str) -> SituationKind {
    match value {
        "ci_debug" => SituationKind::CiDebug,
        "code_change" => SituationKind::CodeChange,
        "git_mutation" => SituationKind::GitMutation,
        "pipeline_control" => SituationKind::PipelineControl,
        "research" => SituationKind::Research,
        "world_model_task" => SituationKind::WorldModelTask,
        "high_risk" => SituationKind::HighRisk,
        "simple_question" => SituationKind::SimpleQuestion,
        "ambiguous" => SituationKind::Ambiguous,
        _ => SituationKind::Greeting,
    }
}
