use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::cozo_guard::run_script_guarded;
use crate::{CognitiveError, TickReport};

pub(crate) fn store_tick_report(
    db: &DbInstance,
    report: &TickReport,
) -> Result<(), CognitiveError> {
    let errors_json = serde_json::to_string(&report.errors)?;
    let mut params = BTreeMap::new();
    params.insert("tick_id".into(), DataValue::from(report.tick_id.as_str()));
    params.insert(
        "dead_letters_replayed".into(),
        optional(report.dead_letters_replayed.map(|count| count as i64)),
    );
    params.insert(
        "proposals_evaluated".into(),
        DataValue::from((report.proposals_evaluated + report.proposals_generated) as i64),
    );
    params.insert(
        "proposals_auto_applied".into(),
        DataValue::from(report.proposals_auto_applied as i64),
    );
    params.insert(
        "proposals_denied".into(),
        DataValue::from(report.proposals_denied as i64),
    );
    params.insert(
        "self_model_updated".into(),
        optional(report.self_model_updated),
    );
    params.insert("errors_json".into(), DataValue::from(errors_json.as_str()));
    params.insert(
        "duration_ms".into(),
        DataValue::from(report.duration_ms as i64),
    );
    params.insert(
        "created_at".into(),
        DataValue::from(report.created_at.to_rfc3339().as_str()),
    );
    run_script_guarded(
        db,
        "?[tick_id, dead_letters_replayed, proposals_evaluated, proposals_auto_applied, proposals_denied, self_model_updated, errors_json, duration_ms, created_at] <- \
         [[$tick_id, $dead_letters_replayed, $proposals_evaluated, $proposals_auto_applied, $proposals_denied, $self_model_updated, $errors_json, $duration_ms, $created_at]]
         :put cognitive_tick_audit { tick_id => dead_letters_replayed, proposals_evaluated, proposals_auto_applied, proposals_denied, self_model_updated, errors_json, duration_ms, created_at }",
        params,
        ScriptMutability::Mutable,
        "store cognitive tick audit",
    )?;
    Ok(())
}

/// A step that produced no measurement is stored as `null`, never as a zero or
/// a `false` that a later reader would mistake for an observation.
fn optional<T: Into<DataValue>>(value: Option<T>) -> DataValue {
    value.map_or(DataValue::Null, Into::into)
}
