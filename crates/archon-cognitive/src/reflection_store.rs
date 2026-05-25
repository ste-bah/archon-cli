use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::cozo_guard::run_script_guarded;
use crate::{CognitiveError, ReflectionRecord};

pub(crate) fn put_reflection(
    db: &DbInstance,
    reflection: &ReflectionRecord,
) -> Result<(), CognitiveError> {
    put_reflection_current(db, reflection).or_else(|_| put_reflection_legacy(db, reflection))
}

pub(crate) fn query_reflection_lessons(db: &DbInstance) -> Result<Vec<String>, CognitiveError> {
    query_reflection_lessons_current(db).or_else(|_| query_reflection_lessons_legacy(db))
}

pub(crate) fn append_ledger(
    dir: &Path,
    reflection: &ReflectionRecord,
) -> Result<(), CognitiveError> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join("cognitive-reflections.jsonl");
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(reflection)?)?;
    Ok(())
}

fn put_reflection_current(
    db: &DbInstance,
    reflection: &ReflectionRecord,
) -> Result<(), CognitiveError> {
    let mut params = reflection_params(reflection);
    params.insert(
        "should_propose".into(),
        DataValue::from(reflection.should_propose),
    );
    run_script_guarded(
        db,
        "?[reflection_id, session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at] <- \
         [[$reflection_id, $session_id, $turn_number, $decision_id, $situation_kind, $attempted, $worked, $failed, $outcome, $lesson, $should_propose, $proposed_rule_id, $created_at]]
         :put cognitive_reflections { reflection_id => session_id, turn_number, decision_id, situation_kind, attempted, worked, failed, outcome, lesson, should_propose, proposed_rule_id, created_at }",
        params,
        ScriptMutability::Mutable,
        "put cognitive reflection",
    )?;
    Ok(())
}

fn put_reflection_legacy(
    db: &DbInstance,
    reflection: &ReflectionRecord,
) -> Result<(), CognitiveError> {
    run_script_guarded(
        db,
        "?[reflection_id, session_id, turn_number, decision_id, outcome, lesson, proposed_rule_id, created_at] <- \
         [[$reflection_id, $session_id, $turn_number, $decision_id, $outcome, $lesson, $proposed_rule_id, $created_at]]
         :put cognitive_reflections { reflection_id => session_id, turn_number, decision_id, outcome, lesson, proposed_rule_id, created_at }",
        reflection_params(reflection),
        ScriptMutability::Mutable,
        "put legacy cognitive reflection",
    )?;
    Ok(())
}

fn query_reflection_lessons_current(db: &DbInstance) -> Result<Vec<String>, CognitiveError> {
    let rows = run_script_guarded(
        db,
        "?[reflection_id, stored_lesson] := *cognitive_reflections{reflection_id, lesson: stored_lesson}",
        Default::default(),
        ScriptMutability::Immutable,
        "query cognitive reflection lessons",
    )?;
    Ok(lesson_rows(rows))
}

fn query_reflection_lessons_legacy(db: &DbInstance) -> Result<Vec<String>, CognitiveError> {
    let rows = run_script_guarded(
        db,
        "?[reflection_id, stored_lesson] := *cognitive_reflections{reflection_id, lesson: stored_lesson}",
        Default::default(),
        ScriptMutability::Immutable,
        "query legacy cognitive reflection lessons",
    )?;
    Ok(lesson_rows(rows))
}

fn lesson_rows(rows: cozo::NamedRows) -> Vec<String> {
    rows.rows
        .iter()
        .map(|row| row[1].get_str().unwrap_or("").to_string())
        .collect()
}

fn reflection_params(reflection: &ReflectionRecord) -> BTreeMap<String, DataValue> {
    let mut params = BTreeMap::new();
    params.insert(
        "reflection_id".into(),
        DataValue::from(reflection.reflection_id.as_str()),
    );
    params.insert(
        "session_id".into(),
        DataValue::from(reflection.session_id.as_str()),
    );
    params.insert(
        "turn_number".into(),
        DataValue::from(reflection.turn_number as i64),
    );
    params.insert(
        "decision_id".into(),
        DataValue::from(reflection.decision_id.as_str()),
    );
    params.insert(
        "situation_kind".into(),
        DataValue::from(reflection.situation_kind.as_str()),
    );
    params.insert(
        "attempted".into(),
        DataValue::from(reflection.attempted.as_str()),
    );
    params.insert("worked".into(), DataValue::from(reflection.worked.as_str()));
    params.insert("failed".into(), DataValue::from(reflection.failed.as_str()));
    params.insert(
        "outcome".into(),
        DataValue::from(reflection.outcome.as_str()),
    );
    params.insert("lesson".into(), DataValue::from(reflection.lesson.as_str()));
    params.insert(
        "proposed_rule_id".into(),
        DataValue::from(reflection.proposed_rule_id.as_deref().unwrap_or("")),
    );
    params.insert(
        "created_at".into(),
        DataValue::from(reflection.created_at.to_rfc3339().as_str()),
    );
    params
}
