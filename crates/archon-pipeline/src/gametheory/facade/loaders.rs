use std::collections::{BTreeMap, HashMap, HashSet};

use cozo::DbInstance;

use super::super::errors::GameTheoryError;
use super::super::fingerprint::GameTheoryFingerprint;
use super::super::routing::RoutingDecision;
use super::super::schema::ensure_gametheory_schema;
use super::types::StoredRunState;

pub(super) fn load_run_state(
    db: &DbInstance,
    run_id: &str,
) -> Result<StoredRunState, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[situation, started_at, status] := *gt_runs{run_id, situation, started_at, completed_at, status}, \
             run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query gt_runs failed: {e}"),
        })?;
    rows.rows
        .first()
        .map(|row| StoredRunState {
            situation: row[0].get_str().unwrap_or("").to_string(),
            started_at: row[1].get_str().unwrap_or("").to_string(),
            status: row[2].get_str().unwrap_or("").to_string(),
        })
        .ok_or_else(|| GameTheoryError::Storage {
            message: format!("run not found: {run_id}"),
        })
}

pub(super) fn load_run_situation(db: &DbInstance, run_id: &str) -> Result<String, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[situation] := *gt_runs{run_id, situation, started_at, completed_at, status}, \
             run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query gt_runs failed: {e}"),
        })?;
    rows.rows
        .first()
        .and_then(|row| row[0].get_str())
        .map(str::to_string)
        .ok_or_else(|| GameTheoryError::Storage {
            message: format!("run not found: {run_id}"),
        })
}

pub(super) fn load_stored_fingerprint(
    db: &DbInstance,
    run_id: &str,
) -> Result<GameTheoryFingerprint, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[fingerprint_json] := *gt_fingerprints{run_id, fingerprint_json, \
             primary_family, created_at}, run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query gt_fingerprints failed: {e}"),
        })?;
    let json = rows
        .rows
        .first()
        .and_then(|row| row[0].get_str())
        .ok_or_else(|| GameTheoryError::Storage {
            message: format!("fingerprint not found for run: {run_id}"),
        })?;
    serde_json::from_str(json).map_err(|e| GameTheoryError::FingerprintParse {
        message: e.to_string(),
    })
}

pub(super) fn load_stored_routing(
    db: &DbInstance,
    run_id: &str,
) -> Result<Option<RoutingDecision>, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[fingerprint_id, enabled, skipped, conditions, created_at] := \
             *gt_routing_decisions{run_id, fingerprint_id, enabled_specialists_json: enabled, \
             skipped_specialists_json: skipped, evaluated_conditions_json: conditions, created_at}, \
             run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query gt_routing_decisions failed: {e}"),
        })?;
    let Some(row) = rows.rows.first() else {
        return Ok(None);
    };

    let enabled = serde_json::from_str(row[1].get_str().unwrap_or("[]")).map_err(|e| {
        GameTheoryError::Storage {
            message: format!("parse enabled specialists failed: {e}"),
        }
    })?;
    let skipped = serde_json::from_str(row[2].get_str().unwrap_or("[]")).map_err(|e| {
        GameTheoryError::Storage {
            message: format!("parse skipped specialists failed: {e}"),
        }
    })?;
    let conditions = serde_json::from_str(row[3].get_str().unwrap_or("[]")).map_err(|e| {
        GameTheoryError::Storage {
            message: format!("parse evaluated conditions failed: {e}"),
        }
    })?;

    Ok(Some(RoutingDecision {
        run_id: run_id.to_string(),
        fingerprint_id: row[0].get_str().unwrap_or("").to_string(),
        enabled_specialists: enabled,
        skipped_specialists: skipped,
        evaluated_conditions: conditions,
        created_at: row[4].get_str().unwrap_or("").to_string(),
    }))
}

pub(super) fn load_completed_specialist_keys(
    db: &DbInstance,
    run_id: &str,
) -> Result<HashSet<String>, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[agent_key] := *gt_specialist_outputs{run_id, agent_key, status}, \
             run_id = $rid, status = 'completed'",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query completed specialists failed: {e}"),
        })?;
    Ok(rows
        .rows
        .iter()
        .filter_map(|row| row[0].get_str().map(str::to_string))
        .collect())
}

pub(super) fn load_completed_specialist_outputs(
    db: &DbInstance,
    run_id: &str,
) -> Result<HashMap<String, String>, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[agent_key, output] := *gt_specialist_outputs{run_id, agent_key, output_json: output, status}, \
             run_id = $rid, status = 'completed'",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query completed specialist outputs failed: {e}"),
        })?;
    Ok(rows
        .rows
        .iter()
        .filter_map(|row| {
            let key = row[0].get_str()?;
            let output = row[1].get_str()?;
            Some((key.to_string(), output.to_string()))
        })
        .collect())
}

pub(super) fn load_specialist_cost_total(
    db: &DbInstance,
    run_id: &str,
) -> Result<f64, GameTheoryError> {
    ensure_gametheory_schema(db).map_err(|e| GameTheoryError::Storage {
        message: e.to_string(),
    })?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), cozo::DataValue::from(run_id));
    let rows = db
        .run_script(
            "?[cost] := *gt_specialist_outputs{run_id, agent_key, cost_usd: cost}, run_id = $rid",
            params,
            cozo::ScriptMutability::Immutable,
        )
        .map_err(|e| GameTheoryError::Storage {
            message: format!("query specialist costs failed: {e}"),
        })?;
    rows.rows.iter().try_fold(0.0, |total, row| {
        let cost = row[0].get_str().unwrap_or("0");
        cost.parse::<f64>()
            .map(|parsed| total + parsed)
            .map_err(|e| GameTheoryError::Storage {
                message: format!("parse specialist cost '{cost}' failed: {e}"),
            })
    })
}

pub(super) fn summarize_output(output: &str) -> String {
    let summary: String = output
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect();
    if output.chars().count() > summary.chars().count() {
        format!("{summary}...")
    } else {
        summary
    }
}
