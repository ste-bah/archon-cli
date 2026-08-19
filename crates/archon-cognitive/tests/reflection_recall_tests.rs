//! Issue #81(a)(b): unresolved reflections reach later turns, bounded, and
//! their reuse is measured rather than assumed.

use std::collections::BTreeSet;

use archon_cognitive::reflection_recall::{
    MAX_INJECTED_REFLECTIONS, MAX_INJECTIONS_PER_REFLECTION, ReflectionRecall, ScoredTurn,
    cited_reflection_ids, render_block,
};
use archon_cognitive::self_model::prediction::TurnVerification;
use archon_cognitive::{MetricEventStore, SituationKind};
use archon_policy::CognitivePolicy;
use cozo::{DbInstance, ScriptMutability};

const SESSION: &str = "session-1";

fn db() -> DbInstance {
    let db = DbInstance::new("mem", "", "").unwrap();
    archon_cognitive::ensure_cognitive_schema(&db).unwrap();
    db
}

fn policy() -> Option<CognitivePolicy> {
    Some(CognitivePolicy {
        enabled: true,
        ..CognitivePolicy::default()
    })
}

/// Write a triggered reflection exactly as `ReflectionWriter::reflect_triggered`
/// does: a row in `cognitive_reflections` plus its provenance row.
fn write_reflection(db: &DbInstance, id: &str, kind: SituationKind, created_at: &str) {
    let script = format!(
        "?[reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at] <- \
         [['{id}', '{SESSION}', 1, 'decision-{id}', '{}', '', '', '', 'failure', 'lesson for {id}', false, '', '{created_at}']]
         :put cognitive_reflections {{ reflection_id => session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at }}",
        kind.as_str()
    );
    db.run_script(&script, Default::default(), ScriptMutability::Mutable)
        .unwrap();
    let script = format!(
        "?[reflection_id, trigger, confidence, evidence_refs_json, created_at] <- \
         [['{id}', 'repeated_tool_failure', 0.75, '[]', '{created_at}']]
         :put cognitive_reflection_evidence {{ reflection_id => trigger, confidence, evidence_refs_json, created_at }}"
    );
    db.run_script(&script, Default::default(), ScriptMutability::Mutable)
        .unwrap();
}

fn passed_turn(
    recall: &ReflectionRecall<'_>,
    turn: u64,
    reflections: &[archon_cognitive::UnresolvedReflection],
    cited: BTreeSet<String>,
) -> archon_cognitive::ReflectionReuseTally {
    recall
        .record_outcome(
            &ScoredTurn {
                session_id: SESSION,
                turn_number: turn,
                model_id: "test-model",
                situation_kind: SituationKind::CodeChange,
            },
            reflections,
            &cited,
            TurnVerification::Passed,
        )
        .unwrap()
}

// ── relevance ────────────────────────────────────────────────

/// "Inject only unresolved/relevant reflections": a lesson drawn from a git
/// mutation says nothing about a research turn.
#[test]
fn only_reflections_from_the_same_situation_kind_are_offered() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    write_reflection(
        &db,
        "r-code",
        SituationKind::CodeChange,
        "2026-01-01T00:00:00Z",
    );
    write_reflection(
        &db,
        "r-git",
        SituationKind::GitMutation,
        "2026-01-02T00:00:00Z",
    );
    let recall = ReflectionRecall::new(&db, dir.path(), policy()).unwrap();

    let offered = recall
        .unresolved_for_turn(SESSION, SituationKind::CodeChange)
        .unwrap();

    assert_eq!(
        offered
            .iter()
            .map(|reflection| reflection.reflection_id.as_str())
            .collect::<Vec<_>>(),
        vec!["r-code"]
    );
}

/// A reflection with no provenance row was never triggered, so it is not a
/// high-value lesson and must not be served as one.
#[test]
fn an_untriggered_reflection_is_not_offered() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    db.run_script(
        "?[reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at] <- \
         [['plain', 'session-1', 1, 'd', 'code_change', '', '', '', 'failure', 'a lesson', false, '', '2026-01-01T00:00:00Z']]
         :put cognitive_reflections { reflection_id => session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at }",
        Default::default(),
        ScriptMutability::Mutable,
    )
    .unwrap();

    assert!(
        ReflectionRecall::new(&db, dir.path(), policy())
            .unwrap()
            .unresolved_for_turn(SESSION, SituationKind::CodeChange)
            .unwrap()
            .is_empty()
    );
}

// ── bounds ───────────────────────────────────────────────────

/// Two independent bounds, because either alone leaks: the per-turn cap keeps
/// one turn's block small, and the per-session cap keeps a long session from
/// re-serving the same unresolved lesson on every turn of it.
#[test]
fn injection_is_bounded_per_turn_and_per_session() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    for index in 0..8 {
        write_reflection(
            &db,
            &format!("r{index}"),
            SituationKind::CodeChange,
            &format!("2026-01-0{}T00:00:00Z", index + 1),
        );
    }
    let recall = ReflectionRecall::new(&db, dir.path(), policy()).unwrap();

    let first = recall
        .unresolved_for_turn(SESSION, SituationKind::CodeChange)
        .unwrap();
    assert_eq!(first.len(), MAX_INJECTED_REFLECTIONS);
    // Newest first: r7, r6, r5.
    assert_eq!(first[0].reflection_id, "r7");

    // Exhaust one reflection's session budget and it drops out of the pool even
    // though it is still unresolved.
    for turn in 1..=MAX_INJECTIONS_PER_REFLECTION as u64 {
        recall.record_injection(SESSION, turn, &first[..1]).unwrap();
    }
    let after = recall
        .unresolved_for_turn(SESSION, SituationKind::CodeChange)
        .unwrap();
    assert_eq!(after.len(), MAX_INJECTED_REFLECTIONS);
    assert!(
        after
            .iter()
            .all(|reflection| reflection.reflection_id != "r7"),
        "an exhausted reflection kept being injected: {after:?}"
    );

    // The budget is per session: a different session starts fresh.
    let other = recall
        .unresolved_for_turn("session-2", SituationKind::CodeChange)
        .unwrap();
    assert_eq!(other[0].reflection_id, "r7");
}

// ── citation is not reuse ────────────────────────────────────

/// The distinction issue #81(b) turns on. A cited lesson on a turn whose
/// verification did not pass is a mention, not evidence that the lesson helped,
/// and it must not retire the reflection.
#[test]
fn a_citation_without_a_verified_pass_is_not_reuse() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    write_reflection(&db, "r1", SituationKind::CodeChange, "2026-01-01T00:00:00Z");
    let recall = ReflectionRecall::new(&db, dir.path(), policy()).unwrap();
    let offered = recall
        .unresolved_for_turn(SESSION, SituationKind::CodeChange)
        .unwrap();
    recall.record_injection(SESSION, 2, &offered).unwrap();

    let tally = recall
        .record_outcome(
            &ScoredTurn {
                session_id: SESSION,
                turn_number: 2,
                model_id: "test-model",
                situation_kind: SituationKind::CodeChange,
            },
            &offered,
            &BTreeSet::from(["r1".to_string()]),
            TurnVerification::Failed,
        )
        .unwrap();

    assert_eq!(tally.cited, 1);
    assert_eq!(tally.verified_reuse, 0);
    assert_eq!(
        recall
            .unresolved_for_turn(SESSION, SituationKind::CodeChange)
            .unwrap()
            .len(),
        1,
        "a citation retired the reflection"
    );
}

/// An uncited lesson on a passing turn is not reuse either: the turn succeeded
/// without it as far as anything can tell.
#[test]
fn a_verified_pass_without_a_citation_is_not_reuse() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    write_reflection(&db, "r1", SituationKind::CodeChange, "2026-01-01T00:00:00Z");
    let recall = ReflectionRecall::new(&db, dir.path(), policy()).unwrap();
    let offered = recall
        .unresolved_for_turn(SESSION, SituationKind::CodeChange)
        .unwrap();
    recall.record_injection(SESSION, 2, &offered).unwrap();

    let tally = passed_turn(&recall, 2, &offered, BTreeSet::new());

    assert_eq!(tally.cited, 0);
    assert_eq!(tally.verified_reuse, 0);
}

/// Cited *and* verified: only then is the lesson resolved and taken out of the
/// pool.
#[test]
fn verified_reuse_resolves_the_reflection() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    write_reflection(&db, "r1", SituationKind::CodeChange, "2026-01-01T00:00:00Z");
    let recall = ReflectionRecall::new(&db, dir.path(), policy()).unwrap();
    let offered = recall
        .unresolved_for_turn(SESSION, SituationKind::CodeChange)
        .unwrap();
    recall.record_injection(SESSION, 2, &offered).unwrap();

    let tally = passed_turn(&recall, 2, &offered, BTreeSet::from(["r1".to_string()]));

    assert_eq!(tally.verified_reuse, 1);
    assert!(
        recall
            .unresolved_for_turn(SESSION, SituationKind::CodeChange)
            .unwrap()
            .is_empty()
    );
}

// ── measurement ──────────────────────────────────────────────

/// The two rates are separate numbers over the same events, so a run where every
/// lesson is cited and none of them helps is visibly different from one where
/// they do.
#[test]
fn citation_and_verified_reuse_derive_as_separate_metrics() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    for index in 0..4 {
        write_reflection(
            &db,
            &format!("r{index}"),
            SituationKind::CodeChange,
            &format!("2026-01-0{}T00:00:00Z", index + 1),
        );
    }
    let recall = ReflectionRecall::new(&db, dir.path(), policy()).unwrap();
    let offered = recall
        .unresolved_for_turn(SESSION, SituationKind::CodeChange)
        .unwrap();
    recall.record_injection(SESSION, 5, &offered).unwrap();
    // All three cited; the turn failed, so none of them counts as reuse.
    recall
        .record_outcome(
            &ScoredTurn {
                session_id: SESSION,
                turn_number: 5,
                model_id: "test-model",
                situation_kind: SituationKind::CodeChange,
            },
            &offered,
            &offered
                .iter()
                .map(|reflection| reflection.reflection_id.clone())
                .collect(),
            TurnVerification::Failed,
        )
        .unwrap();

    let snapshot = MetricEventStore::new(&db, dir.path())
        .unwrap()
        .latest_snapshot()
        .unwrap();
    assert_eq!(
        snapshot.pooled("lesson_citation_rate").unwrap().value,
        Some(1.0)
    );
    assert_eq!(
        snapshot
            .pooled("reflection_verified_reuse_rate")
            .unwrap()
            .value,
        Some(0.0),
        "citation was counted as reuse"
    );
}

// ── the prompt block ─────────────────────────────────────────

#[test]
fn an_empty_pool_renders_no_block() {
    assert!(render_block(&[]).is_none());
}

/// Citation is an exact marker match: a turn that happens to repeat the lesson's
/// wording has not cited it.
#[test]
fn citation_requires_the_marker_not_the_wording() {
    let db = db();
    let dir = tempfile::tempdir().unwrap();
    write_reflection(&db, "r1", SituationKind::CodeChange, "2026-01-01T00:00:00Z");
    let recall = ReflectionRecall::new(&db, dir.path(), policy()).unwrap();
    let offered = recall
        .unresolved_for_turn(SESSION, SituationKind::CodeChange)
        .unwrap();
    let block = render_block(&offered).expect("block");

    assert!(block.contains(&offered[0].marker));
    assert!(block.contains("lesson for r1"));
    assert!(cited_reflection_ids("lesson for r1, as it happens", &offered).is_empty());
    assert_eq!(
        cited_reflection_ids(&format!("applying [{}]", offered[0].marker), &offered),
        BTreeSet::from(["r1".to_string()])
    );
}
