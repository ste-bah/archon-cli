use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use cozo::DbInstance;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::reflection_store::{append_ledger, put_reflection, query_reflection_lessons};
use crate::schema::ensure_cognitive_schema;
use crate::{CognitiveError, DecisionRecord, SituationKind, VerificationVerdict};

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
        if reflection.should_propose {
            if let Err(error) = self.lesson_sink.promote_lesson(&reflection) {
                degraded.push(format!("lesson_promotion_failed:{error}"));
            }
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
