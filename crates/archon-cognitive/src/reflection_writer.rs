use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use cozo::DbInstance;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::reflection_store::{
    append_ledger, put_reflection, put_reflection_evidence, query_reflection_lessons,
};
use crate::reflection_trigger::{ReflectionTrigger, TriggeredReflection};
use crate::schema::ensure_cognitive_schema;
use crate::{
    CandidateActionKind, CognitiveError, DecisionRecord, SituationKind, VerificationVerdict,
};

/// Longest accepted evidence reference.
///
/// Evidence refs are identifiers (`kind:uuid`), so anything long is not a
/// reference. Together with the whitespace rule this is what stops a caller
/// from smuggling narrative text — model reasoning included — into the one
/// free-form-looking field on the record.
const MAX_EVIDENCE_REF_LEN: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeSummary {
    Success,
    PartialSuccess,
    Failure,
    UserCorrected,
    Degraded,
}

impl OutcomeSummary {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PartialSuccess => "partial_success",
            Self::Failure => "failure",
            Self::UserCorrected => "user_corrected",
            Self::Degraded => "degraded",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReflectInput {
    pub decision: DecisionRecord,
    pub situation_kind: SituationKind,
    pub verification: VerificationVerdict,
    pub outcome: OutcomeSummary,
    pub user_corrected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReflectionRecord {
    pub reflection_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub decision_id: String,
    pub situation_kind: SituationKind,
    pub attempted: String,
    pub worked: String,
    pub failed: String,
    pub lesson: String,
    pub should_propose: bool,
    pub proposed_rule_id: Option<String>,
    pub outcome: OutcomeSummary,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReflectionWriteOutcome {
    pub reflection: Option<ReflectionRecord>,
    pub degraded: Vec<String>,
}

pub trait LessonSink: Clone {
    fn promote_lesson(&self, reflection: &ReflectionRecord) -> Result<(), CognitiveError>;
}

#[derive(Debug, Clone, Default)]
pub struct NoopLessonSink;

impl LessonSink for NoopLessonSink {
    fn promote_lesson(&self, _reflection: &ReflectionRecord) -> Result<(), CognitiveError> {
        Ok(())
    }
}

pub struct ReflectionWriter<'a, S = NoopLessonSink> {
    db: &'a DbInstance,
    ledger_dir: PathBuf,
    record_enabled: bool,
    similarity_threshold: usize,
    lesson_sink: S,
}

impl<'a> ReflectionWriter<'a, NoopLessonSink> {
    pub fn new(
        db: &'a DbInstance,
        ledger_dir: impl AsRef<Path>,
        record_enabled: bool,
    ) -> Result<Self, CognitiveError> {
        Self::with_lesson_sink(db, ledger_dir, record_enabled, NoopLessonSink)
    }
}

impl<'a, S: LessonSink> ReflectionWriter<'a, S> {
    pub fn with_lesson_sink(
        db: &'a DbInstance,
        ledger_dir: impl AsRef<Path>,
        record_enabled: bool,
        lesson_sink: S,
    ) -> Result<Self, CognitiveError> {
        ensure_cognitive_schema(db)?;
        Ok(Self {
            db,
            ledger_dir: ledger_dir.as_ref().to_path_buf(),
            record_enabled,
            similarity_threshold: 3,
            lesson_sink,
        })
    }

    pub fn reflect(&self, input: ReflectInput) -> Result<ReflectionWriteOutcome, CognitiveError> {
        if !self.record_enabled || input.decision.decision_id.is_empty() {
            return Ok(ReflectionWriteOutcome::default());
        }
        if !is_meaningful(&input) {
            return Ok(ReflectionWriteOutcome::default());
        }

        let mut reflection = build_reflection(&input);
        reflection.should_propose = self.is_recurring_lesson(&reflection.lesson)?;
        let mut degraded = Vec::new();
        if let Err(error) = put_reflection(self.db, &reflection) {
            degraded.push(format!("cozo_reflection_write_failed:{error}"));
        }
        if let Err(error) = append_ledger(&self.ledger_dir, &reflection) {
            degraded.push(format!("reflection_ledger_write_failed:{error}"));
        }
        if reflection.should_propose
            && let Err(error) = self.lesson_sink.promote_lesson(&reflection)
        {
            degraded.push(format!("lesson_promotion_failed:{error}"));
        }
        Ok(ReflectionWriteOutcome {
            reflection: Some(reflection),
            degraded,
        })
    }

    /// Write a reflection because a live turn tripped a trigger.
    ///
    /// This is the path issue #81 asks for, and it is deliberately narrower
    /// than [`Self::reflect`]: it persists only the goal, the mismatch, the
    /// proposed adjustment, the evidence references and the confidence. There
    /// is no parameter that can carry raw chain-of-thought, and the strings on
    /// the record are composed here from enums and counts rather than accepted
    /// from the caller.
    pub fn reflect_triggered(
        &self,
        input: TriggeredReflectInput,
    ) -> Result<ReflectionWriteOutcome, CognitiveError> {
        if !self.record_enabled || input.decision_id.is_empty() {
            return Ok(ReflectionWriteOutcome::default());
        }
        let mut input = input;
        let (evidence_refs, rejected) =
            sanitize_evidence_refs(std::mem::take(&mut input.evidence_refs));
        let mut degraded = Vec::new();
        if rejected > 0 {
            // The count, never the content: reporting what was rejected would
            // reintroduce exactly the text this filter exists to keep out.
            degraded.push(format!("evidence_refs_rejected:{rejected}"));
        }

        let mut reflection = build_triggered_reflection(&input);
        reflection.should_propose = self.is_recurring_lesson(&reflection.lesson)?;
        if let Err(error) = put_reflection(self.db, &reflection) {
            degraded.push(format!("cozo_reflection_write_failed:{error}"));
        }
        if let Err(error) = put_reflection_evidence(
            self.db,
            &reflection.reflection_id,
            input.trigger.trigger.as_str(),
            input.trigger.confidence,
            &evidence_refs,
            &reflection.created_at.to_rfc3339(),
        ) {
            degraded.push(format!("reflection_evidence_write_failed:{error}"));
        }
        if let Err(error) = append_ledger(&self.ledger_dir, &reflection) {
            degraded.push(format!("reflection_ledger_write_failed:{error}"));
        }
        if reflection.should_propose
            && let Err(error) = self.lesson_sink.promote_lesson(&reflection)
        {
            degraded.push(format!("lesson_promotion_failed:{error}"));
        }
        Ok(ReflectionWriteOutcome {
            reflection: Some(reflection),
            degraded,
        })
    }

    fn is_recurring_lesson(&self, lesson: &str) -> Result<bool, CognitiveError> {
        let key = normalize_lesson(lesson);
        let count = query_reflection_lessons(self.db)?
            .iter()
            .filter(|stored| normalize_lesson(stored) == key)
            .count();
        Ok(count + 1 >= self.similarity_threshold)
    }
}

/// Everything the triggered path is allowed to know about a turn.
///
/// Ids, enums and a bounded confidence. No user text, no assistant text, no
/// tool output — so "never persist raw chain-of-thought" is enforced by the
/// type rather than by reviewer discipline.
#[derive(Debug, Clone, PartialEq)]
pub struct TriggeredReflectInput {
    pub decision_id: String,
    pub session_id: String,
    pub turn_number: u64,
    pub situation_kind: SituationKind,
    /// What the plan intended, i.e. the goal this reflection is measured
    /// against.
    pub goal_action: Option<CandidateActionKind>,
    /// What the turn actually did.
    pub observed_action: Option<CandidateActionKind>,
    pub trigger: TriggeredReflection,
    pub evidence_refs: Vec<String>,
}

/// Keep only refs that look like references.
///
/// Returns the survivors and how many were dropped.
fn sanitize_evidence_refs(refs: Vec<String>) -> (Vec<String>, usize) {
    let total = refs.len();
    let kept: Vec<String> = refs
        .into_iter()
        .filter(|reference| {
            !reference.trim().is_empty()
                && reference.len() <= MAX_EVIDENCE_REF_LEN
                && !reference.chars().any(char::is_whitespace)
        })
        .collect();
    let rejected = total - kept.len();
    (kept, rejected)
}

fn build_triggered_reflection(input: &TriggeredReflectInput) -> ReflectionRecord {
    let kind = input.situation_kind.as_str();
    ReflectionRecord {
        reflection_id: Uuid::new_v4().to_string(),
        session_id: input.session_id.clone(),
        turn_number: input.turn_number,
        decision_id: input.decision_id.clone(),
        situation_kind: input.situation_kind,
        attempted: truncate(format!("goal:{kind}:{}", action_str(input.goal_action))),
        // Nothing was verified on this path, so nothing is claimed to have
        // worked. An empty string here is the honest value.
        worked: String::new(),
        failed: truncate(format!(
            "mismatch:{}:observed_{}",
            input.trigger.trigger.as_str(),
            action_str(input.observed_action)
        )),
        lesson: truncate(triggered_lesson(input.trigger.trigger, kind)),
        should_propose: false,
        proposed_rule_id: None,
        outcome: triggered_outcome(input.trigger.trigger),
        created_at: Utc::now(),
    }
}

/// The proposed adjustment for each trigger.
///
/// Kept generic over the situation kind rather than the turn's content: a
/// lesson that quoted the turn would be both raw text and unusable as a
/// recurring-lesson key.
fn triggered_lesson(trigger: ReflectionTrigger, kind: &str) -> String {
    match trigger {
        ReflectionTrigger::HighConfidenceCorrection => {
            format!("{kind}: user correction lowers confidence and requires source recheck")
        }
        ReflectionTrigger::RepeatedToolFailure => {
            format!("{kind}: repeated tool failure should stop retrying and re-plan the approach")
        }
        ReflectionTrigger::HighSurprise => {
            format!("{kind}: outcome diverged from the plan; record why before claiming completion")
        }
    }
}

fn triggered_outcome(trigger: ReflectionTrigger) -> OutcomeSummary {
    match trigger {
        ReflectionTrigger::HighConfidenceCorrection => OutcomeSummary::UserCorrected,
        ReflectionTrigger::RepeatedToolFailure => OutcomeSummary::Failure,
        ReflectionTrigger::HighSurprise => OutcomeSummary::Degraded,
    }
}

fn action_str(action: Option<CandidateActionKind>) -> &'static str {
    action.map(CandidateActionKind::as_str).unwrap_or("none")
}

fn is_meaningful(input: &ReflectInput) -> bool {
    match input.outcome {
        OutcomeSummary::Failure
        | OutcomeSummary::PartialSuccess
        | OutcomeSummary::UserCorrected
        | OutcomeSummary::Degraded => true,
        OutcomeSummary::Success => {
            input.user_corrected
                || !matches!(
                    input.situation_kind,
                    SituationKind::Greeting | SituationKind::SimpleQuestion
                )
                || !matches!(input.verification, VerificationVerdict::NotRun)
        }
    }
}

fn build_reflection(input: &ReflectInput) -> ReflectionRecord {
    let failed = failed_summary(&input.verification, input.outcome);
    ReflectionRecord {
        reflection_id: Uuid::new_v4().to_string(),
        session_id: input.decision.session_id.clone(),
        turn_number: input.decision.turn_number,
        decision_id: input.decision.decision_id.clone(),
        situation_kind: input.situation_kind,
        attempted: truncate(format!(
            "decision:{} selected:{}",
            input.decision.decision_id, input.decision.selected_candidate_id
        )),
        worked: truncate(worked_summary(&input.verification, input.outcome)),
        failed: truncate(failed),
        lesson: truncate(lesson_summary(input)),
        should_propose: false,
        proposed_rule_id: None,
        outcome: input.outcome,
        created_at: Utc::now(),
    }
}

fn worked_summary(verification: &VerificationVerdict, outcome: OutcomeSummary) -> String {
    match (outcome, verification) {
        (OutcomeSummary::Success, VerificationVerdict::Passed) => "verified_success".into(),
        (OutcomeSummary::Success, _) => "completed_with_unverified_evidence".into(),
        (OutcomeSummary::PartialSuccess, _) => "partial_progress_recorded".into(),
        _ => String::new(),
    }
}

fn failed_summary(verification: &VerificationVerdict, outcome: OutcomeSummary) -> String {
    match verification {
        VerificationVerdict::Failed { reason } => format!("verification_failed:{reason}"),
        VerificationVerdict::Skipped { reason } => format!("verification_skipped:{reason}"),
        _ if matches!(outcome, OutcomeSummary::Failure) => "outcome_failed".into(),
        _ if matches!(outcome, OutcomeSummary::Degraded) => "outcome_degraded".into(),
        _ if matches!(outcome, OutcomeSummary::UserCorrected) => "user_correction".into(),
        _ => String::new(),
    }
}

fn lesson_summary(input: &ReflectInput) -> String {
    let kind = input.situation_kind.as_str();
    match (&input.verification, input.outcome, input.user_corrected) {
        (VerificationVerdict::Failed { .. }, _, _) => {
            format!("{kind}: require passing verification evidence before completion")
        }
        (VerificationVerdict::Skipped { .. }, _, _) => {
            format!("{kind}: record explicit not_run reason before claiming confidence")
        }
        (_, OutcomeSummary::UserCorrected, _) | (_, _, true) => {
            format!("{kind}: user correction lowers confidence and requires source recheck")
        }
        (_, OutcomeSummary::Degraded, _) => {
            format!("{kind}: degraded dependency should trigger fallback and audit note")
        }
        _ => format!("{kind}: repeat compact verified action pattern"),
    }
}

fn normalize_lesson(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn truncate(value: String) -> String {
    const MAX: usize = 240;
    if value.len() <= MAX {
        value
    } else {
        value.chars().take(MAX).collect()
    }
}
