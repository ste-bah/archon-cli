use std::collections::BTreeMap;

use cozo::{DataValue, DbInstance, ScriptMutability};

use crate::errors::{ProvenanceError, Result};
use crate::record::{ProvenanceEdge, ProvenanceEdgeType, ProvenanceRecord};

pub fn ensure_schema(db: &DbInstance) -> Result<()> {
    run_create(
        db,
        r#":create prov_records {
            record_id: String =>
            artifact_id: String,
            artifact_type: String,
            operation: String,
            input_hashes_json: String,
            output_hash: String,
            parent_record_ids_json: String,
            tool_name: String,
            agent_name: String,
            model: String,
            parameters_json: String,
            timestamp: String,
            chain_hash: String
        }"#,
    )?;
    run_create(
        db,
        r#":create prov_edges {
            edge_id: String =>
            from_artifact_id: String,
            to_artifact_id: String,
            edge_type: String,
            created_at: String
        }"#,
    )?;
    Ok(())
}

pub fn insert_record(db: &DbInstance, record: &ProvenanceRecord) -> Result<()> {
    ensure_schema(db)?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), DataValue::from(record.record_id.as_str()));
    params.insert("aid".into(), DataValue::from(record.artifact_id.as_str()));
    params.insert("typ".into(), DataValue::from(record.artifact_type.as_str()));
    params.insert("op".into(), DataValue::from(record.operation.as_str()));
    params.insert(
        "inputs".into(),
        DataValue::from(serde_json::to_string(&record.input_hashes)?.as_str()),
    );
    params.insert("out".into(), DataValue::from(record.output_hash.as_str()));
    params.insert(
        "parents".into(),
        DataValue::from(serde_json::to_string(&record.parent_record_ids)?.as_str()),
    );
    params.insert(
        "tool".into(),
        DataValue::from(record.tool_name.as_deref().unwrap_or("")),
    );
    params.insert(
        "agent".into(),
        DataValue::from(record.agent_name.as_deref().unwrap_or("")),
    );
    params.insert(
        "model".into(),
        DataValue::from(record.model.as_deref().unwrap_or("")),
    );
    params.insert(
        "params".into(),
        DataValue::from(record.parameters_json.to_string().as_str()),
    );
    params.insert("ts".into(), DataValue::from(record.timestamp.as_str()));
    params.insert("chain".into(), DataValue::from(record.chain_hash.as_str()));
    db.run_script(
        r#"
        ?[record_id, artifact_id, artifact_type, operation, input_hashes_json,
          output_hash, parent_record_ids_json, tool_name, agent_name, model,
          parameters_json, timestamp, chain_hash] <- [[$rid, $aid, $typ, $op,
          $inputs, $out, $parents, $tool, $agent, $model, $params, $ts, $chain]]
        :put prov_records {
            record_id => artifact_id, artifact_type, operation, input_hashes_json,
            output_hash, parent_record_ids_json, tool_name, agent_name, model,
            parameters_json, timestamp, chain_hash
        }
        "#,
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| ProvenanceError::Store(format!("insert provenance record failed: {e}")))?;
    Ok(())
}

pub fn insert_edge(db: &DbInstance, edge: &ProvenanceEdge) -> Result<()> {
    ensure_schema(db)?;
    let mut params = BTreeMap::new();
    params.insert("eid".into(), DataValue::from(edge.edge_id.as_str()));
    params.insert(
        "from".into(),
        DataValue::from(edge.from_artifact_id.as_str()),
    );
    params.insert("to".into(), DataValue::from(edge.to_artifact_id.as_str()));
    params.insert("typ".into(), DataValue::from(edge.edge_type.as_str()));
    params.insert("ts".into(), DataValue::from(edge.created_at.as_str()));
    db.run_script(
        r#"
        ?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at]
            <- [[$eid, $from, $to, $typ, $ts]]
        :put prov_edges { edge_id => from_artifact_id, to_artifact_id, edge_type, created_at }
        "#,
        params,
        ScriptMutability::Mutable,
    )
    .map_err(|e| ProvenanceError::Store(format!("insert provenance edge failed: {e}")))?;
    Ok(())
}

pub fn get_record(db: &DbInstance, record_id: &str) -> Result<Option<ProvenanceRecord>> {
    ensure_schema(db)?;
    let mut params = BTreeMap::new();
    params.insert("rid".into(), DataValue::from(record_id));
    let script = record_query("record_id = $rid");
    let result = db
        .run_script(&script, params, ScriptMutability::Immutable)
        .map_err(|e| ProvenanceError::Store(format!("get provenance record failed: {e}")))?;
    result
        .rows
        .first()
        .map(|row| row_to_record(row))
        .transpose()
}

pub fn get_record_by_artifact(
    db: &DbInstance,
    artifact_id: &str,
) -> Result<Option<ProvenanceRecord>> {
    ensure_schema(db)?;
    let mut params = BTreeMap::new();
    params.insert("aid".into(), DataValue::from(artifact_id));
    let script = record_query("artifact_id = $aid");
    let result = db
        .run_script(&script, params, ScriptMutability::Immutable)
        .map_err(|e| ProvenanceError::Store(format!("get artifact record failed: {e}")))?;
    result
        .rows
        .first()
        .map(|row| row_to_record(row))
        .transpose()
}

pub fn list_edges_from(db: &DbInstance, artifact_id: &str) -> Result<Vec<ProvenanceEdge>> {
    query_edges(
        db,
        "prov_edges",
        "from_artifact_id = $artifact",
        artifact_id,
        "list provenance edges from failed",
    )
}

pub fn list_edges_to(db: &DbInstance, artifact_id: &str) -> Result<Vec<ProvenanceEdge>> {
    query_edges(
        db,
        "prov_edges",
        "to_artifact_id = $artifact",
        artifact_id,
        "list provenance edges to failed",
    )
}

pub fn row_to_edge(row: &[DataValue]) -> ProvenanceEdge {
    ProvenanceEdge {
        edge_id: str_col(row, 0),
        from_artifact_id: str_col(row, 1),
        to_artifact_id: str_col(row, 2),
        edge_type: ProvenanceEdgeType::parse(&str_col(row, 3)),
        created_at: str_col(row, 4),
    }
}

fn query_edges(
    db: &DbInstance,
    relation: &str,
    filter: &str,
    artifact_id: &str,
    label: &str,
) -> Result<Vec<ProvenanceEdge>> {
    let mut params = BTreeMap::new();
    params.insert("artifact".into(), DataValue::from(artifact_id));
    let script = format!(
        "?[edge_id, from_artifact_id, to_artifact_id, edge_type, created_at] := \
         *{relation}{{edge_id, from_artifact_id, to_artifact_id, edge_type, created_at}}, {filter}"
    );
    match db.run_script(&script, params, ScriptMutability::Immutable) {
        Ok(result) => Ok(result.rows.iter().map(|row| row_to_edge(row)).collect()),
        Err(e) if relation_missing(&e.to_string()) => Ok(Vec::new()),
        Err(e) => Err(ProvenanceError::Store(format!("{label}: {e}"))),
    }
}

fn record_query(filter: &str) -> String {
    format!(
        "?[record_id, artifact_id, artifact_type, operation, input_hashes_json, \
         output_hash, parent_record_ids_json, tool_name, agent_name, model, \
         parameters_json, timestamp, chain_hash] := *prov_records{{record_id, \
         artifact_id, artifact_type, operation, input_hashes_json, output_hash, \
         parent_record_ids_json, tool_name, agent_name, model, parameters_json, \
         timestamp, chain_hash}}, {filter}"
    )
}

fn row_to_record(row: &[DataValue]) -> Result<ProvenanceRecord> {
    Ok(ProvenanceRecord {
        record_id: str_col(row, 0),
        artifact_id: str_col(row, 1),
        artifact_type: str_col(row, 2),
        operation: str_col(row, 3),
        input_hashes: serde_json::from_str(&str_col(row, 4))?,
        output_hash: str_col(row, 5),
        parent_record_ids: serde_json::from_str(&str_col(row, 6))?,
        tool_name: non_empty(row, 7),
        agent_name: non_empty(row, 8),
        model: non_empty(row, 9),
        parameters_json: serde_json::from_str(&str_col(row, 10))?,
        timestamp: str_col(row, 11),
        chain_hash: str_col(row, 12),
    })
}

fn run_create(db: &DbInstance, script: &str) -> Result<()> {
    match db.run_script(script, Default::default(), ScriptMutability::Mutable) {
        Ok(_) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("already exists") || msg.contains("conflicts with an existing") {
                Ok(())
            } else {
                Err(ProvenanceError::Schema(msg))
            }
        }
    }
}

fn str_col(row: &[DataValue], idx: usize) -> String {
    row.get(idx)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

fn non_empty(row: &[DataValue], idx: usize) -> Option<String> {
    let value = str_col(row, idx);
    (!value.is_empty()).then_some(value)
}

pub fn relation_missing(message: &str) -> bool {
    message.contains("Cannot find requested stored relation")
        || message.contains("not found")
        || message.contains("does not exist")
}
