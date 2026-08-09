//! Persistence for immutable evaluation-window definitions.

use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::CognitiveError;
use crate::cozo_guard::run_script_guarded;
use crate::metrics::codec::{int_col, json_col, str_col, time_col};
use crate::metrics::window::{CohortRole, EvaluationWindow};

const WINDOW_COLUMNS: &str = "evaluation_window_id, label, started_at, ended_at, population_query_version, segmentation_keys_json, cohort_role, cohort_identity, metric_definition_version, created_at";

/// Outcome of declaring a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowDeclaration {
    /// First declaration; the definition is now frozen.
    Declared,
    /// Byte-identical redeclaration, accepted as a no-op so a restarted
    /// process can safely reassert the windows it expects to exist.
    AlreadyDeclared,
}

pub(crate) fn declare_window(
    db: &DbInstance,
    window: &EvaluationWindow,
) -> Result<WindowDeclaration, CognitiveError> {
    window.validate()?;
    if let Some(existing) = load_window(db, &window.evaluation_window_id)? {
        if &existing == window {
            return Ok(WindowDeclaration::AlreadyDeclared);
        }
        return Err(CognitiveError::Metric(format!(
            "evaluation window `{}` is immutable and cannot be redefined",
            window.evaluation_window_id
        )));
    }
    run_script_guarded(
        db,
        &format!(
            "?[{WINDOW_COLUMNS}] <- [[$evaluation_window_id, $label, $started_at, $ended_at, $population_query_version, $segmentation_keys_json, $cohort_role, $cohort_identity, $metric_definition_version, $created_at]]
             :put cognitive_evaluation_windows {{ evaluation_window_id => label, started_at, ended_at, population_query_version, segmentation_keys_json, cohort_role, cohort_identity, metric_definition_version, created_at }}"
        ),
        window_params(window)?,
        ScriptMutability::Mutable,
        "declare cognitive evaluation window",
    )?;
    Ok(WindowDeclaration::Declared)
}

pub(crate) fn load_window(
    db: &DbInstance,
    evaluation_window_id: &str,
) -> Result<Option<EvaluationWindow>, CognitiveError> {
    Ok(list_windows(db)?
        .into_iter()
        .find(|window| window.evaluation_window_id == evaluation_window_id))
}

pub(crate) fn list_windows(db: &DbInstance) -> Result<Vec<EvaluationWindow>, CognitiveError> {
    let rows = run_script_guarded(
        db,
        &format!("?[{WINDOW_COLUMNS}] := *cognitive_evaluation_windows{{{WINDOW_COLUMNS}}}"),
        Default::default(),
        ScriptMutability::Immutable,
        "list cognitive evaluation windows",
    )?;
    let mut windows = rows
        .rows
        .iter()
        .map(|row| row_to_window(row))
        .collect::<Result<Vec<_>, _>>()?;
    windows.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.evaluation_window_id.cmp(&right.evaluation_window_id))
    });
    Ok(windows)
}

fn row_to_window(row: &[DataValue]) -> Result<EvaluationWindow, CognitiveError> {
    let cohort_role = str_col(row, 6);
    Ok(EvaluationWindow {
        evaluation_window_id: str_col(row, 0),
        label: str_col(row, 1),
        started_at: time_col(row, 2)?,
        ended_at: time_col(row, 3)?,
        population_query_version: int_col(row, 4),
        segmentation_keys: json_col(row, 5)?,
        cohort_role: cohort_role
            .parse()
            .map_err(|()| CognitiveError::Metric(format!("unknown cohort role `{cohort_role}`")))?,
        cohort_identity: str_col(row, 7),
        metric_definition_version: int_col(row, 8),
        created_at: time_col(row, 9)?,
    })
}

fn window_params(window: &EvaluationWindow) -> Result<BTreeMap<String, DataValue>, CognitiveError> {
    let mut params = BTreeMap::new();
    params.insert(
        "evaluation_window_id".into(),
        DataValue::from(window.evaluation_window_id.as_str()),
    );
    params.insert("label".into(), DataValue::from(window.label.as_str()));
    params.insert(
        "started_at".into(),
        DataValue::from(window.started_at.to_rfc3339().as_str()),
    );
    params.insert(
        "ended_at".into(),
        DataValue::from(window.ended_at.to_rfc3339().as_str()),
    );
    params.insert(
        "population_query_version".into(),
        DataValue::from(window.population_query_version),
    );
    params.insert(
        "segmentation_keys_json".into(),
        DataValue::from(serde_json::to_string(&window.segmentation_keys)?.as_str()),
    );
    params.insert(
        "cohort_role".into(),
        DataValue::from(CohortRole::as_str(window.cohort_role)),
    );
    params.insert(
        "cohort_identity".into(),
        DataValue::from(window.cohort_identity.as_str()),
    );
    params.insert(
        "metric_definition_version".into(),
        DataValue::from(window.metric_definition_version),
    );
    params.insert(
        "created_at".into(),
        DataValue::from(window.created_at.to_rfc3339().as_str()),
    );
    Ok(params)
}
