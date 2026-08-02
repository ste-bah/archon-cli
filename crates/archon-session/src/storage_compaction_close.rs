use std::collections::BTreeMap;

use chrono::Utc;
use cozo::DataValue;

use super::storage_compaction::{
    CompactionLedgerRecord, CompactionSegment, CompactionSummaryStatus, CompactionTelemetryRecord,
    segment_in_transaction, telemetry_params,
};
use super::storage_compaction_codec::{SEGMENT_PUT, ledger_params, segment_params};
use super::{SessionError, SessionStore, db_err, extract_str};

impl SessionStore {
    pub fn close_compaction_segment(
        &self,
        session_id: &str,
        start_index: u64,
        end_index: u64,
        body: &[String],
    ) -> Result<CompactionSegment, SessionError> {
        self.close_compaction_segment_with_records(
            session_id,
            start_index,
            end_index,
            body,
            &[],
            None,
        )
    }

    pub fn close_compaction_segment_with_records(
        &self,
        session_id: &str,
        start_index: u64,
        end_index: u64,
        body: &[String],
        ledger: &[CompactionLedgerRecord],
        telemetry: Option<&CompactionTelemetryRecord>,
    ) -> Result<CompactionSegment, SessionError> {
        self.get_session(session_id)?;
        #[cfg(test)]
        if self
            .delete_before_compaction_close_transaction
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            self.delete_session(session_id)?;
        }
        let now = Utc::now().to_rfc3339();
        let segment = CompactionSegment {
            id: format!("segment:{session_id}:{start_index}:{end_index}"),
            session_id: session_id.to_string(),
            start_index,
            end_index,
            summary_status: CompactionSummaryStatus::Pending,
            summary: None,
            summary_model: None,
            summary_attribution: None,
            summary_failure: None,
            summary_input_tokens: None,
            summary_output_tokens: None,
            summary_cost: None,
            created_at: now.clone(),
            updated_at: now,
        };
        self.put_compaction_segment_records(&segment, body, ledger, telemetry)
    }

    fn put_compaction_segment_records(
        &self,
        segment: &CompactionSegment,
        body: &[String],
        ledger: &[CompactionLedgerRecord],
        telemetry: Option<&CompactionTelemetryRecord>,
    ) -> Result<CompactionSegment, SessionError> {
        let transaction = self.db.multi_transaction(true);
        let result = (|| {
            require_session_in_transaction(&transaction, &segment.session_id)?;
            if let Some(existing) = existing_matching_segment(&transaction, segment, body)? {
                return Ok(existing);
            }
            transaction
                .run_script(SEGMENT_PUT, segment_params(segment))
                .map_err(db_err)?;
            put_body(&transaction, &segment.id, body)?;
            #[cfg(test)]
            if self
                .fail_next_compaction_close_after_body
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(SessionError::DbError(
                    "injected compaction close failure after body".into(),
                ));
            }
            put_ledger_and_telemetry(&transaction, ledger, telemetry)?;
            #[cfg(test)]
            if self
                .fail_next_compaction_close_after_records
                .swap(false, std::sync::atomic::Ordering::SeqCst)
            {
                return Err(SessionError::DbError(
                    "injected compaction close failure after records".into(),
                ));
            }
            Ok(segment.clone())
        })();
        match result {
            Ok(segment) => {
                transaction.commit().map_err(db_err)?;
                Ok(segment)
            }
            Err(error) => {
                let _ = transaction.abort();
                Err(error)
            }
        }
    }
}

fn existing_matching_segment(
    transaction: &cozo::MultiTransaction,
    segment: &CompactionSegment,
    body: &[String],
) -> Result<Option<CompactionSegment>, SessionError> {
    let Some(existing) = segment_in_transaction(transaction, &segment.id)? else {
        return Ok(None);
    };
    if body_in_transaction(transaction, &segment.id)?.as_deref() != Some(body) {
        return Err(SessionError::DbError(format!(
            "compaction segment '{}' already exists with different source body",
            segment.id
        )));
    }
    Ok(Some(existing))
}

fn put_ledger_and_telemetry(
    transaction: &cozo::MultiTransaction,
    ledger: &[CompactionLedgerRecord],
    telemetry: Option<&CompactionTelemetryRecord>,
) -> Result<(), SessionError> {
    for record in ledger {
        transaction
            .run_script(
                "?[id, session_id, kind, payload, start_index, end_index, created_at] <- [[$id, $session_id, $kind, $payload, $start_index, $end_index, $created_at]]
                 :put compaction_ledger {id => session_id, kind, payload, start_index, end_index, created_at}",
                ledger_params(record),
            )
            .map_err(db_err)?;
    }
    if let Some(record) = telemetry {
        transaction
            .run_script(
                "?[id, session_id, action, payload, created_at] <- [[$id, $session_id, $action, $payload, $created_at]]
                 :put compaction_telemetry {id => session_id, action, payload, created_at}",
                telemetry_params(record),
            )
            .map_err(db_err)?;
    }
    Ok(())
}

fn require_session_in_transaction(
    transaction: &cozo::MultiTransaction,
    session_id: &str,
) -> Result<(), SessionError> {
    let mut params = BTreeMap::new();
    params.insert("sid".into(), DataValue::from(session_id));
    let rows = transaction
        .run_script("?[id] := *sessions{id}, id = $sid", params)
        .map_err(db_err)?;
    if rows.rows.is_empty() {
        return Err(SessionError::NotFound(format!(
            "session '{session_id}' not found"
        )));
    }
    Ok(())
}

fn put_body(
    transaction: &cozo::MultiTransaction,
    id: &str,
    body: &[String],
) -> Result<(), SessionError> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(id));
    params.insert(
        "body".into(),
        DataValue::from(
            serde_json::to_string(body)
                .map_err(|error| SessionError::DbError(error.to_string()))?,
        ),
    );
    transaction
        .run_script(
            "?[id, body] <- [[$id, $body]] :put compaction_segment_bodies {id => body}",
            params,
        )
        .map_err(db_err)?;
    Ok(())
}

fn body_in_transaction(
    transaction: &cozo::MultiTransaction,
    id: &str,
) -> Result<Option<Vec<String>>, SessionError> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(id));
    let rows = transaction
        .run_script(
            "?[body] := *compaction_segment_bodies{id, body}, id = $id",
            params,
        )
        .map_err(db_err)?;
    rows.rows
        .first()
        .map(|row| {
            serde_json::from_str(&extract_str(&row[0]))
                .map_err(|error| SessionError::DbError(error.to_string()))
        })
        .transpose()
}
