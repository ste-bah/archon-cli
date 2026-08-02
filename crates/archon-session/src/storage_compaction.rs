use std::collections::BTreeMap;

use chrono::Utc;
use cozo::{DataValue, ScriptMutability};

use super::storage_compaction_codec::{
    SEGMENT_PUT, ledger_params, segment_from_row, segment_params,
};
use super::{SessionError, SessionStore, db_err, extract_i64, extract_str};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionSummaryStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Redacted,
}

impl CompactionSummaryStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Redacted => "redacted",
        }
    }

    pub(super) fn parse(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "redacted" => Self::Redacted,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CompactionSegment {
    pub id: String,
    pub session_id: String,
    pub start_index: u64,
    pub end_index: u64,
    pub summary_status: CompactionSummaryStatus,
    pub summary: Option<String>,
    pub summary_model: Option<String>,
    pub summary_attribution: Option<String>,
    pub summary_failure: Option<String>,
    pub summary_input_tokens: Option<u64>,
    pub summary_output_tokens: Option<u64>,
    pub summary_cost: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompactionLedgerRecord {
    pub id: String,
    pub session_id: String,
    pub kind: String,
    pub payload: String,
    pub source_start_index: u64,
    pub source_end_index: u64,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompactionTelemetryRecord {
    pub id: String,
    pub session_id: String,
    pub action: String,
    pub payload: String,
    pub created_at: String,
}

impl SessionStore {
    pub fn get_compaction_segment(
        &self,
        id: &str,
    ) -> Result<Option<CompactionSegment>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("id".into(), DataValue::from(id));
        let result = self.db.run_script(
            "?[id, session_id, start_index, end_index, status, summary, model, attribution, failure, input_tokens, output_tokens, cost, created_at, updated_at] :=
             *compaction_segments{id, session_id, start_index, end_index, status, summary, model, attribution, failure, input_tokens, output_tokens, cost, created_at, updated_at}, id = $id",
            params,
            ScriptMutability::Immutable,
        ).map_err(db_err)?;
        Ok(result.rows.first().map(|row| segment_from_row(row)))
    }

    pub fn list_compaction_segments(
        &self,
        session_id: &str,
    ) -> Result<Vec<CompactionSegment>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(session_id));
        let result = self.db.run_script(
            "?[id, session_id, start_index, end_index, status, summary, model, attribution, failure, input_tokens, output_tokens, cost, created_at, updated_at] :=
             *compaction_segments{id, session_id, start_index, end_index, status, summary, model, attribution, failure, input_tokens, output_tokens, cost, created_at, updated_at}, session_id = $sid :sort start_index",
            params,
            ScriptMutability::Immutable,
        ).map_err(db_err)?;
        Ok(result
            .rows
            .iter()
            .map(|row| segment_from_row(row))
            .collect())
    }

    pub fn load_compaction_segment_body(&self, id: &str) -> Result<Vec<String>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("id".into(), DataValue::from(id));
        let result = self
            .db
            .run_script(
                "?[body] := *compaction_segment_bodies{id, body}, id = $id",
                params,
                ScriptMutability::Immutable,
            )
            .map_err(db_err)?;
        let Some(row) = result.rows.first() else {
            return Err(SessionError::NotFound(id.to_string()));
        };
        serde_json::from_str(&extract_str(&row[0]))
            .map_err(|error| SessionError::DbError(error.to_string()))
    }

    pub fn load_authorized_compaction_segment_body(
        &self,
        session_id: &str,
        id: &str,
    ) -> Result<Vec<String>, SessionError> {
        let segment = self
            .get_compaction_segment(id)?
            .filter(|segment| segment.session_id == session_id)
            .ok_or_else(|| SessionError::NotFound(id.to_string()))?;
        if segment
            .summary_failure
            .as_deref()
            .is_some_and(|failure| failure.starts_with("redacted:"))
        {
            return Err(SessionError::NotFound(id.to_string()));
        }
        self.load_compaction_segment_body(id)
    }

    pub fn claim_compaction_segment_summary(
        &self,
        id: &str,
        model: &str,
        attribution: &str,
    ) -> Result<Option<String>, SessionError> {
        let transaction = self.db.multi_transaction(true);
        let Some(mut segment) = segment_in_transaction(&transaction, id)? else {
            let _ = transaction.abort();
            return Err(SessionError::NotFound(id.to_string()));
        };
        if !matches!(
            segment.summary_status,
            CompactionSummaryStatus::Pending | CompactionSummaryStatus::Failed
        ) {
            let _ = transaction.abort();
            return Ok(None);
        }
        let claim = encode_summary_claim(attribution);
        segment.summary_status = CompactionSummaryStatus::Running;
        segment.summary_model = Some(model.to_string());
        segment.summary_attribution = Some(claim.clone());
        segment.summary_failure = None;
        segment.updated_at = Utc::now().to_rfc3339();
        transaction
            .run_script(SEGMENT_PUT, segment_params(&segment))
            .map_err(db_err)?;
        transaction.commit().map_err(db_err)?;
        Ok(Some(claim))
    }

    pub fn complete_compaction_segment_summary(
        &self,
        id: &str,
        claim: &str,
        summary: &str,
        input_tokens: u64,
        output_tokens: u64,
        cost: f64,
    ) -> Result<bool, SessionError> {
        let transaction = self.db.multi_transaction(true);
        let Some(mut segment) = segment_in_transaction(&transaction, id)? else {
            let _ = transaction.abort();
            return Err(SessionError::NotFound(id.to_string()));
        };
        if segment.summary_status != CompactionSummaryStatus::Running
            || segment.summary_attribution.as_deref() != Some(claim)
        {
            let _ = transaction.abort();
            return Ok(false);
        }
        segment.summary_status = CompactionSummaryStatus::Succeeded;
        segment.summary = Some(summary.to_string());
        segment.summary_input_tokens = Some(input_tokens);
        segment.summary_output_tokens = Some(output_tokens);
        segment.summary_cost = Some(cost);
        segment.summary_failure = None;
        segment.updated_at = Utc::now().to_rfc3339();
        transaction
            .run_script(SEGMENT_PUT, segment_params(&segment))
            .map_err(db_err)?;
        transaction.commit().map_err(db_err)?;
        Ok(true)
    }

    pub fn fail_compaction_segment_summary(
        &self,
        id: &str,
        claim: &str,
        failure: &str,
    ) -> Result<bool, SessionError> {
        let transaction = self.db.multi_transaction(true);
        let Some(mut segment) = segment_in_transaction(&transaction, id)? else {
            let _ = transaction.abort();
            return Err(SessionError::NotFound(id.to_string()));
        };
        if segment.summary_status != CompactionSummaryStatus::Running
            || segment.summary_attribution.as_deref() != Some(claim)
        {
            let _ = transaction.abort();
            return Ok(false);
        }
        segment.summary_status = CompactionSummaryStatus::Failed;
        segment.summary_failure = Some(failure.to_string());
        segment.updated_at = Utc::now().to_rfc3339();
        transaction
            .run_script(SEGMENT_PUT, segment_params(&segment))
            .map_err(db_err)?;
        transaction.commit().map_err(db_err)?;
        Ok(true)
    }

    pub fn put_compaction_ledger_record(
        &self,
        record: &CompactionLedgerRecord,
    ) -> Result<(), SessionError> {
        self.db
            .run_mutable(
                "?[id, session_id, kind, payload, start_index, end_index, created_at] <- [[$id, $session_id, $kind, $payload, $start_index, $end_index, $created_at]]
                 :put compaction_ledger {id => session_id, kind, payload, start_index, end_index, created_at}",
                ledger_params(record),
                "session store: put compaction ledger record",
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn list_compaction_ledger_records(
        &self,
        session_id: &str,
    ) -> Result<Vec<CompactionLedgerRecord>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(session_id));
        let result = self.db.run_script(
            "?[id, session_id, kind, payload, start_index, end_index, created_at] := *compaction_ledger{id, session_id, kind, payload, start_index, end_index, created_at}, session_id = $sid :sort created_at, id",
            params,
            ScriptMutability::Immutable,
        ).map_err(db_err)?;
        Ok(result
            .rows
            .iter()
            .map(|row| CompactionLedgerRecord {
                id: extract_str(&row[0]),
                session_id: extract_str(&row[1]),
                kind: extract_str(&row[2]),
                payload: extract_str(&row[3]),
                source_start_index: extract_i64(&row[4]).max(0) as u64,
                source_end_index: extract_i64(&row[5]).max(0) as u64,
                created_at: extract_str(&row[6]),
            })
            .collect())
    }

    pub fn put_compaction_telemetry(
        &self,
        record: &CompactionTelemetryRecord,
    ) -> Result<(), SessionError> {
        self.db.run_mutable(
            "?[id, session_id, action, payload, created_at] <- [[$id, $session_id, $action, $payload, $created_at]]
             :put compaction_telemetry {id => session_id, action, payload, created_at}",
            telemetry_params(record),
            "session store: put compaction telemetry record",
        ).map_err(db_err)?;
        Ok(())
    }

    pub fn list_compaction_telemetry(
        &self,
        session_id: &str,
    ) -> Result<Vec<CompactionTelemetryRecord>, SessionError> {
        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(session_id));
        let result = self.db.run_script(
            "?[id, session_id, action, payload, created_at] := *compaction_telemetry{id, session_id, action, payload, created_at}, session_id = $sid :sort created_at, id",
            params,
            ScriptMutability::Immutable,
        ).map_err(db_err)?;
        Ok(result
            .rows
            .iter()
            .map(|row| CompactionTelemetryRecord {
                id: extract_str(&row[0]),
                session_id: extract_str(&row[1]),
                action: extract_str(&row[2]),
                payload: extract_str(&row[3]),
                created_at: extract_str(&row[4]),
            })
            .collect())
    }
}

pub(super) fn segment_in_transaction(
    transaction: &cozo::MultiTransaction,
    id: &str,
) -> Result<Option<CompactionSegment>, SessionError> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(id));
    let result = transaction
        .run_script(
            "?[id, session_id, start_index, end_index, status, summary, model, attribution, failure, input_tokens, output_tokens, cost, created_at, updated_at] :=
             *compaction_segments{id, session_id, start_index, end_index, status, summary, model, attribution, failure, input_tokens, output_tokens, cost, created_at, updated_at}, id = $id",
            params,
        )
        .map_err(db_err)?;
    Ok(result.rows.first().map(|row| segment_from_row(row)))
}

pub(super) fn telemetry_params(record: &CompactionTelemetryRecord) -> BTreeMap<String, DataValue> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(record.id.as_str()));
    params.insert(
        "session_id".into(),
        DataValue::from(record.session_id.as_str()),
    );
    params.insert("action".into(), DataValue::from(record.action.as_str()));
    params.insert("payload".into(), DataValue::from(record.payload.as_str()));
    params.insert(
        "created_at".into(),
        DataValue::from(record.created_at.as_str()),
    );
    params
}

fn encode_summary_claim(attribution: &str) -> String {
    let mut value = serde_json::from_str(attribution)
        .unwrap_or_else(|_| serde_json::json!({"attribution": attribution}));
    value["compaction_claim_id"] = serde_json::json!(uuid::Uuid::new_v4().to_string());
    value.to_string()
}
