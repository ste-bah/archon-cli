//! `Lesson -> DerivedFrom -> Correction + evidence`.
//!
//! The roadmap's R2 edge model ends in a lesson, and until now the attribution
//! row named a decision, an action attempt and a candidate but no lesson, so
//! the last edge was missing. This is it: an accepted attribution derives one
//! causal lesson, stored with the correction and the evidence it rests on.
//!
//! Two rules shape it.
//!
//! **Only an accepted attribution derives a lesson.** A refusal has no cause to
//! generalise from, and minting a lesson anyway would put unexplained
//! corrections into the corpus that later rule proposals generalise over --
//! which is the same defect as attributing to the nearest candidate, one layer
//! further on.
//!
//! **Deduplication is by provenance, structurally.** The roadmap says
//! "deduplicate lessons by embedding similarity plus compatible cause/action
//! class". There are no embeddings in this crate, and adding them for this would
//! be a similarity threshold to tune in a subsystem whose whole point is not
//! guessing. Instead the lesson id IS a hash of its provenance key, so two
//! provenance-compatible lessons cannot help but collide and two incompatible
//! ones cannot be merged by any threshold. A second correction reaching an
//! existing key corroborates it; it does not create a rival row.
//!
//! Related work deliberately not reused: `archon_memory::garden::provenance`
//! has a `provenance_compatible` predicate from the same roadmap family, but it
//! is defined over `archon_memory::types::Memory` and compares memory type,
//! project path and source type. A causal lesson is not a `Memory` row, this
//! crate does not depend on `archon-memory`, and none of those three fields has
//! an analogue here -- a lesson's compatibility is over cause/action class, not
//! over where a memory row came from. Compatibility is therefore defined here,
//! in the same shape, rather than duplicated from there.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use cozo::{DataValue, DbInstance, ScriptMutability};
use serde::{Deserialize, Serialize};

use crate::CognitiveError;
use crate::attribution::input::AttributionInput;
use crate::attribution::{AttributionAssessment, CAUSE_ACTION_CLASS_NONE};
use crate::cozo_guard::run_script_guarded;

/// Identity of the lesson-derivation procedure.
///
/// Part of the lesson id, so a change to how lessons are composed or keyed
/// starts a new population instead of silently corroborating the old one.
pub const CAUSAL_LESSON_VERSION: &str = "causal-lesson/v1";

/// Most corrections whose ids are kept on one lesson's evidence.
///
/// Corroboration count is unbounded; the ref list is not. A lesson corroborated
/// four hundred times does not need four hundred ids to be reviewable, and an
/// unbounded column is how a measurement row becomes a log.
const MAX_EVIDENCE_REFS: usize = 32;

/// Length of the hashed provenance key inside a lesson id.
const PROVENANCE_HASH_LEN: usize = 16;

/// One lesson derived from an attributed correction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CausalLesson {
    pub lesson_id: String,
    /// Fields that decide compatibility, joined. Two lessons with the same key
    /// are the same lesson.
    pub provenance_key: String,
    pub session_id: String,
    pub turn_number: u64,
    pub correction_id: String,
    pub correction_type_code: String,
    pub cause_action_class: String,
    pub cause_label: String,
    pub causal_candidate_id: String,
    pub decision_id: String,
    pub action_attempt_id: String,
    pub task_class: String,
    pub model_id: String,
    /// Composed from enums and identifiers only. No user text, no model text.
    pub lesson: String,
    pub evidence_refs: Vec<String>,
    pub corroboration_count: u64,
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

/// The fields that make two lessons the same lesson.
///
/// Cause/action class and the cause's own label are the roadmap's stated
/// compatibility axis. Correction type is added because "the tool failed" and
/// "the tool was not permitted" are different lessons about the same action.
/// Task class and model are the cohort keys every other measurement in this
/// crate is segmented by, and pooling across them would make a lesson's
/// corroboration count uninterpretable.
fn provenance_key(
    cause_action_class: &str,
    cause_label: &str,
    correction_type_code: &str,
    task_class: &str,
    model_id: &str,
) -> String {
    format!(
        "{CAUSAL_LESSON_VERSION}|{cause_action_class}|{}|{correction_type_code}|{task_class}|{model_id}",
        cause_label.trim().to_lowercase()
    )
}

fn lesson_id_for(provenance_key: &str) -> String {
    let digest = blake3::hash(provenance_key.as_bytes()).to_hex().to_string();
    format!("causal-lesson:{}", &digest[..PROVENANCE_HASH_LEN])
}

/// Derive the lesson an accepted attribution implies.
///
/// `None` for every refusal: an abstention or an unattributed correction names
/// no cause, and a lesson with no cause is a sentence about nothing.
pub fn causal_lesson(
    input: &AttributionInput,
    assessment: &AttributionAssessment,
    task_class: &str,
    model_id: &str,
) -> Option<CausalLesson> {
    let accepted = assessment.accepted_candidate()?;
    let candidate = &accepted.candidate;
    let cause_action_class = candidate.cause_action_class.as_code();
    let correction_type_code = input.correction.correction_type_code.as_str();
    let provenance_key = provenance_key(
        cause_action_class,
        &candidate.label,
        correction_type_code,
        task_class,
        model_id,
    );

    let mut evidence_refs = vec![
        format!("correction:{}", input.correction.correction_id),
        format!("session:{}", input.correction.session_id),
        format!("causal_candidate:{}", candidate.candidate_id),
    ];
    evidence_refs.extend(candidate.evidence_refs());
    evidence_refs.extend(
        accepted
            .evidence_codes()
            .into_iter()
            .map(|code| format!("evidence:{code}")),
    );
    evidence_refs.truncate(MAX_EVIDENCE_REFS);

    Some(CausalLesson {
        lesson_id: lesson_id_for(&provenance_key),
        provenance_key,
        session_id: input.correction.session_id.clone(),
        turn_number: input.correction.turn_number,
        correction_id: input.correction.correction_id.clone(),
        correction_type_code: correction_type_code.to_string(),
        cause_action_class: cause_action_class.to_string(),
        cause_label: candidate.label.clone(),
        causal_candidate_id: candidate.candidate_id.clone(),
        decision_id: candidate.decision_id.clone().unwrap_or_default(),
        action_attempt_id: candidate.action_attempt_id.clone().unwrap_or_default(),
        task_class: task_class.to_string(),
        model_id: model_id.to_string(),
        lesson: format!(
            "A {cause_action_class} of `{}` in a {task_class} turn drew a {correction_type_code} correction.",
            candidate.label
        ),
        evidence_refs,
        corroboration_count: 1,
        first_seen_at: input.correction.recorded_at,
        last_seen_at: input.correction.recorded_at,
    })
}

/// Whether two lessons are the same lesson.
///
/// Equivalent to id equality by construction; kept as a named predicate because
/// "these were merged because their ids collided" is a worse thing to read in
/// six months than "these were merged because their provenance matched".
pub fn provenance_compatible(left: &CausalLesson, right: &CausalLesson) -> bool {
    left.provenance_key == right.provenance_key
}

/// What a lesson write did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LessonWriteOutcome {
    /// No compatible lesson existed.
    Created(String),
    /// A compatible lesson existed and this correction is new evidence for it.
    Corroborated(String),
    /// A compatible lesson already carries this correction; a replay.
    AlreadyCorroborated(String),
}

impl LessonWriteOutcome {
    pub fn lesson_id(&self) -> &str {
        match self {
            Self::Created(id) | Self::Corroborated(id) | Self::AlreadyCorroborated(id) => id,
        }
    }

    pub fn into_lesson_id(self) -> String {
        match self {
            Self::Created(id) | Self::Corroborated(id) | Self::AlreadyCorroborated(id) => id,
        }
    }
}

/// Store `lesson`, corroborating a provenance-compatible one if it exists.
pub fn record_causal_lesson(
    db: &DbInstance,
    lesson: &CausalLesson,
) -> Result<LessonWriteOutcome, CognitiveError> {
    crate::schema::ensure_cognitive_schema(db)?;
    let correction_ref = format!("correction:{}", lesson.correction_id);
    match read_causal_lesson(db, &lesson.lesson_id)? {
        None => {
            put_causal_lesson(db, lesson)?;
            Ok(LessonWriteOutcome::Created(lesson.lesson_id.clone()))
        }
        Some(existing) => {
            if !provenance_compatible(&existing, lesson) {
                // Unreachable while the id is a hash of the key, which is why it
                // is checked: a silent merge of two incompatible lessons is the
                // exact failure the keying is meant to make impossible.
                return Err(CognitiveError::Store(format!(
                    "causal lesson `{}` exists with a different provenance key",
                    lesson.lesson_id
                )));
            }
            if existing.evidence_refs.contains(&correction_ref) {
                return Ok(LessonWriteOutcome::AlreadyCorroborated(existing.lesson_id));
            }
            let mut merged = existing;
            merged.corroboration_count = merged.corroboration_count.saturating_add(1);
            merged.last_seen_at = lesson.last_seen_at.max(merged.last_seen_at);
            merged.evidence_refs.push(correction_ref);
            merged.evidence_refs.truncate(MAX_EVIDENCE_REFS);
            put_causal_lesson(db, &merged)?;
            Ok(LessonWriteOutcome::Corroborated(merged.lesson_id))
        }
    }
}

const LESSON_COLUMNS: &str = "lesson_id, provenance_key, session_id, turn_number, correction_id, \
     correction_type, cause_action_class, cause_label, causal_candidate_id, decision_id, \
     action_attempt_id, task_class, model_id, lesson, evidence_refs_json, corroboration_count, \
     first_seen_at, last_seen_at";

fn put_causal_lesson(db: &DbInstance, lesson: &CausalLesson) -> Result<(), CognitiveError> {
    let mut params = BTreeMap::new();
    params.insert(
        "lesson_id".into(),
        DataValue::from(lesson.lesson_id.as_str()),
    );
    params.insert(
        "provenance_key".into(),
        DataValue::from(lesson.provenance_key.as_str()),
    );
    params.insert(
        "session_id".into(),
        DataValue::from(lesson.session_id.as_str()),
    );
    params.insert(
        "turn_number".into(),
        DataValue::from(lesson.turn_number as i64),
    );
    params.insert(
        "correction_id".into(),
        DataValue::from(lesson.correction_id.as_str()),
    );
    params.insert(
        "correction_type".into(),
        DataValue::from(lesson.correction_type_code.as_str()),
    );
    params.insert(
        "cause_action_class".into(),
        DataValue::from(lesson.cause_action_class.as_str()),
    );
    params.insert(
        "cause_label".into(),
        DataValue::from(lesson.cause_label.as_str()),
    );
    params.insert(
        "causal_candidate_id".into(),
        DataValue::from(lesson.causal_candidate_id.as_str()),
    );
    params.insert(
        "decision_id".into(),
        DataValue::from(lesson.decision_id.as_str()),
    );
    params.insert(
        "action_attempt_id".into(),
        DataValue::from(lesson.action_attempt_id.as_str()),
    );
    params.insert(
        "task_class".into(),
        DataValue::from(lesson.task_class.as_str()),
    );
    params.insert("model_id".into(), DataValue::from(lesson.model_id.as_str()));
    params.insert("lesson".into(), DataValue::from(lesson.lesson.as_str()));
    params.insert(
        "evidence_refs_json".into(),
        DataValue::from(serde_json::to_string(&lesson.evidence_refs)?.as_str()),
    );
    params.insert(
        "corroboration_count".into(),
        DataValue::from(lesson.corroboration_count as i64),
    );
    params.insert(
        "first_seen_at".into(),
        DataValue::from(lesson.first_seen_at.to_rfc3339().as_str()),
    );
    params.insert(
        "last_seen_at".into(),
        DataValue::from(lesson.last_seen_at.to_rfc3339().as_str()),
    );
    run_script_guarded(
        db,
        &format!(
            "?[{LESSON_COLUMNS}] <- [[$lesson_id, $provenance_key, $session_id, $turn_number, \
             $correction_id, $correction_type, $cause_action_class, $cause_label, \
             $causal_candidate_id, $decision_id, $action_attempt_id, $task_class, $model_id, \
             $lesson, $evidence_refs_json, $corroboration_count, $first_seen_at, $last_seen_at]]
             :put cognitive_causal_lessons {{ lesson_id => provenance_key, session_id, \
             turn_number, correction_id, correction_type, cause_action_class, cause_label, \
             causal_candidate_id, decision_id, action_attempt_id, task_class, model_id, lesson, \
             evidence_refs_json, corroboration_count, first_seen_at, last_seen_at }}"
        ),
        params,
        ScriptMutability::Mutable,
        "put causal lesson",
    )?;
    Ok(())
}

/// Read one lesson back, for corroboration and for source-of-truth inspection.
pub fn read_causal_lesson(
    db: &DbInstance,
    lesson_id: &str,
) -> Result<Option<CausalLesson>, CognitiveError> {
    let mut params = BTreeMap::new();
    params.insert("lesson_id".into(), DataValue::from(lesson_id));
    let rows = run_script_guarded(
        db,
        &format!(
            "?[{LESSON_COLUMNS}] := *cognitive_causal_lessons{{{LESSON_COLUMNS}}}, \
             lesson_id = $lesson_id"
        ),
        params,
        ScriptMutability::Immutable,
        "read causal lesson",
    )?;
    rows.rows.first().map(|row| row_to_lesson(row)).transpose()
}

/// Every stored causal lesson, newest corroboration first.
pub fn causal_lessons(db: &DbInstance) -> Result<Vec<CausalLesson>, CognitiveError> {
    let rows = run_script_guarded(
        db,
        &format!("?[{LESSON_COLUMNS}] := *cognitive_causal_lessons{{{LESSON_COLUMNS}}}"),
        Default::default(),
        ScriptMutability::Immutable,
        "list causal lessons",
    )?;
    let mut lessons = rows
        .rows
        .iter()
        .map(|row| row_to_lesson(row))
        .collect::<Result<Vec<_>, _>>()?;
    lessons.sort_by(|left, right| {
        right
            .last_seen_at
            .cmp(&left.last_seen_at)
            .then_with(|| left.lesson_id.cmp(&right.lesson_id))
    });
    Ok(lessons)
}

fn row_to_lesson(row: &[DataValue]) -> Result<CausalLesson, CognitiveError> {
    let text = |index: usize| row[index].get_str().unwrap_or("").to_string();
    let number = |index: usize| row[index].get_int().unwrap_or_default().max(0) as u64;
    let time = |index: usize| {
        DateTime::parse_from_rfc3339(row[index].get_str().unwrap_or(""))
            .map(|parsed| parsed.with_timezone(&Utc))
            .unwrap_or_else(|_| DateTime::<Utc>::MIN_UTC)
    };
    Ok(CausalLesson {
        lesson_id: text(0),
        provenance_key: text(1),
        session_id: text(2),
        turn_number: number(3),
        correction_id: text(4),
        correction_type_code: text(5),
        cause_action_class: text(6),
        cause_label: text(7),
        causal_candidate_id: text(8),
        decision_id: text(9),
        action_attempt_id: text(10),
        task_class: text(11),
        model_id: text(12),
        lesson: text(13),
        evidence_refs: serde_json::from_str(row[14].get_str().unwrap_or("[]"))?,
        corroboration_count: number(15),
        first_seen_at: time(16),
        last_seen_at: time(17),
    })
}

/// `cause_action_class` recorded on a lesson-free attribution row.
///
/// Re-exported here so a reader of this module can see that a lesson and a
/// no-cause verdict use one vocabulary.
pub const LESSON_ABSENT_CAUSE_CLASS: &str = CAUSE_ACTION_CLASS_NONE;
