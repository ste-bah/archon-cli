use std::sync::{Arc, Mutex};

use archon_cognitive::{
    DecisionRecord, LessonSink, OutcomeSummary, ReflectInput, ReflectionRecord, ReflectionWriter,
    SituationKind, VerificationVerdict,
};
use chrono::Utc;
use cozo::{DbInstance, ScriptMutability};

#[derive(Clone, Default)]
struct RecordingSink {
    lessons: Arc<Mutex<Vec<String>>>,
}

impl LessonSink for RecordingSink {
    fn promote_lesson(
        &self,
        reflection: &ReflectionRecord,
    ) -> Result<(), archon_cognitive::CognitiveError> {
        self.lessons.lock().unwrap().push(reflection.lesson.clone());
        Ok(())
    }
}

fn decision(id: &str, turn_number: u64) -> DecisionRecord {
    DecisionRecord {
        decision_id: id.into(),
        situation_id: "situation-1".into(),
        session_id: "session-1".into(),
        turn_number,
        selected_candidate_id: "candidate-1".into(),
        rejected_alternatives: Vec::new(),
        heuristic_scores: Vec::new(),
        policy_verdict: None,
        verification_contract: Some("contract-1".into()),
        user_visible_summary: "compact summary".into(),
        created_at: Utc::now(),
    }
}

fn input(
    id: &str,
    kind: SituationKind,
    outcome: OutcomeSummary,
    verification: VerificationVerdict,
) -> ReflectInput {
    ReflectInput {
        decision: decision(id, 1),
        situation_kind: kind,
        verification,
        outcome,
        user_corrected: false,
    }
}

#[test]
fn failure_reflection_writes_cozo_and_jsonl() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let writer = ReflectionWriter::new(&db, dir.path(), true).unwrap();
    let outcome = writer
        .reflect(input(
            "decision-1",
            SituationKind::CodeChange,
            OutcomeSummary::Failure,
            VerificationVerdict::Failed {
                reason: "tests failed".into(),
            },
        ))
        .unwrap();

    assert!(outcome.degraded.is_empty());
    assert_eq!(reflection_count(&db), 1);
    let ledger = std::fs::read_to_string(dir.path().join("cognitive-reflections.jsonl")).unwrap();
    assert!(ledger.contains("verification_failed"));
}

#[test]
fn trivial_success_is_skipped() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let writer = ReflectionWriter::new(&db, dir.path(), true).unwrap();
    let outcome = writer
        .reflect(input(
            "decision-1",
            SituationKind::Greeting,
            OutcomeSummary::Success,
            VerificationVerdict::NotRun,
        ))
        .unwrap();

    assert!(outcome.reflection.is_none());
    assert_eq!(reflection_count(&db), 0);
}

#[test]
fn repeated_lessons_flag_proposal_and_promote_lesson() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let sink = RecordingSink::default();
    let writer = ReflectionWriter::with_lesson_sink(&db, dir.path(), true, sink.clone()).unwrap();

    for idx in 0..3 {
        let outcome = writer
            .reflect(input(
                &format!("decision-{idx}"),
                SituationKind::CiDebug,
                OutcomeSummary::Failure,
                VerificationVerdict::Skipped {
                    reason: "CI logs unavailable".into(),
                },
            ))
            .unwrap();
        if idx < 2 {
            assert!(!outcome.reflection.unwrap().should_propose);
        } else {
            assert!(outcome.reflection.unwrap().should_propose);
        }
    }

    assert_eq!(sink.lessons.lock().unwrap().len(), 1);
}

#[test]
fn disabled_or_empty_decision_skips_without_write() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let writer = ReflectionWriter::new(&db, dir.path(), false).unwrap();
    let skipped = writer
        .reflect(input(
            "decision-1",
            SituationKind::CodeChange,
            OutcomeSummary::Failure,
            VerificationVerdict::NotRun,
        ))
        .unwrap();

    assert!(skipped.reflection.is_none());
    assert_eq!(reflection_count(&db), 0);

    let writer = ReflectionWriter::new(&db, dir.path(), true).unwrap();
    let mut empty = input(
        "",
        SituationKind::CodeChange,
        OutcomeSummary::Failure,
        VerificationVerdict::NotRun,
    );
    empty.decision.decision_id.clear();
    assert!(writer.reflect(empty).unwrap().reflection.is_none());
}

#[test]
fn reflection_does_not_store_raw_user_text() {
    let db = DbInstance::new("mem", "", "").unwrap();
    let dir = tempfile::tempdir().unwrap();
    let writer = ReflectionWriter::new(&db, dir.path(), true).unwrap();
    let reflection = writer
        .reflect(input(
            "decision-check",
            SituationKind::Research,
            OutcomeSummary::UserCorrected,
            VerificationVerdict::Passed,
        ))
        .unwrap()
        .reflection
        .unwrap();
    let serialized = serde_json::to_string(&reflection).unwrap();

    assert!(!serialized.contains("what I asked"));
    assert!(!serialized.contains("please delete everything"));
    assert!(serialized.contains("user correction"));
}

fn reflection_count(db: &DbInstance) -> usize {
    let rows = db
        .run_script(
            "?[reflection_id] := *cognitive_reflections{reflection_id}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .unwrap();
    rows.rows.len()
}
