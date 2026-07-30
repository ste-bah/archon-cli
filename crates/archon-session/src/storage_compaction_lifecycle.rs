use std::collections::BTreeMap;

use chrono::Utc;
use cozo::DataValue;

use super::storage_compaction::{
    CompactionSegment, CompactionSummaryStatus, segment_in_transaction,
};
use super::storage_compaction_codec::{SEGMENT_PUT, segment_params};
use super::{SessionError, SessionStore, db_err};

impl SessionStore {
    pub fn recoverable_compaction_segments(
        &self,
        session_id: &str,
    ) -> Result<Vec<CompactionSegment>, SessionError> {
        let segments = self.list_compaction_segments(session_id)?;
        let mut recoverable = Vec::new();
        for segment in segments {
            match segment.summary_status {
                CompactionSummaryStatus::Pending => recoverable.push(segment),
                CompactionSummaryStatus::Running => {
                    if let Some(segment) = self.reset_interrupted_summary(&segment)? {
                        recoverable.push(segment);
                    }
                }
                CompactionSummaryStatus::Failed
                    if segment
                        .summary_failure
                        .as_deref()
                        .is_some_and(is_retryable_provider_failure) =>
                {
                    recoverable.push(segment);
                }
                _ => {}
            }
        }
        Ok(recoverable)
    }

    pub fn mark_compaction_segment_source_invalid(
        &self,
        id: &str,
        failure: &str,
    ) -> Result<(), SessionError> {
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
            return Ok(());
        }
        segment.summary_status = CompactionSummaryStatus::Failed;
        segment.summary_failure = Some(failure.to_string());
        segment.updated_at = Utc::now().to_rfc3339();
        transaction
            .run_script(SEGMENT_PUT, segment_params(&segment))
            .map_err(db_err)?;
        transaction.commit().map_err(db_err)
    }

    pub fn redact_compaction_segment(
        &self,
        session_id: &str,
        id: &str,
        reason: &str,
    ) -> Result<(), SessionError> {
        let transaction = self.db.multi_transaction(true);
        let Some(mut segment) = segment_in_transaction(&transaction, id)? else {
            let _ = transaction.abort();
            return Err(SessionError::NotFound(id.to_string()));
        };
        if segment.session_id != session_id {
            let _ = transaction.abort();
            return Err(SessionError::NotFound(id.to_string()));
        }

        let mut params = BTreeMap::new();
        params.insert("id".into(), DataValue::from(id));
        transaction
            .run_script(
                "?[id] <- [[$id]] :rm compaction_segment_bodies {id}",
                params,
            )
            .map_err(db_err)?;

        let mut params = BTreeMap::new();
        params.insert("sid".into(), DataValue::from(session_id));
        params.insert("start".into(), DataValue::from(segment.start_index as i64));
        params.insert("end".into(), DataValue::from(segment.end_index as i64));
        transaction
            .run_script(
                "?[id] := *compaction_ledger{id, session_id, start_index, end_index}, session_id = $sid, start_index <= $end, end_index >= $start :rm compaction_ledger {id}",
                params,
            )
            .map_err(db_err)?;

        segment.summary_status = CompactionSummaryStatus::Redacted;
        segment.summary = None;
        segment.summary_attribution = None;
        segment.summary_failure = Some(format!("redacted: {reason}"));
        segment.updated_at = Utc::now().to_rfc3339();
        transaction
            .run_script(SEGMENT_PUT, segment_params(&segment))
            .map_err(db_err)?;
        transaction.commit().map_err(db_err)
    }

    #[cfg(test)]
    pub(crate) fn recover_interrupted_compaction_segment(
        &self,
        stale: &CompactionSegment,
    ) -> Result<Option<CompactionSegment>, SessionError> {
        self.reset_interrupted_summary(stale)
    }

    fn reset_interrupted_summary(
        &self,
        stale: &CompactionSegment,
    ) -> Result<Option<CompactionSegment>, SessionError> {
        let transaction = self.db.multi_transaction(true);
        let Some(mut current) = segment_in_transaction(&transaction, &stale.id)? else {
            let _ = transaction.abort();
            return Ok(None);
        };
        if current.summary_status != CompactionSummaryStatus::Running
            || current.summary_attribution != stale.summary_attribution
        {
            let _ = transaction.abort();
            return Ok(
                (current.summary_status == CompactionSummaryStatus::Pending).then_some(current)
            );
        }
        current.summary_status = CompactionSummaryStatus::Pending;
        current.summary_attribution = None;
        current.summary_failure = Some("interrupted before completion".into());
        current.updated_at = Utc::now().to_rfc3339();
        transaction
            .run_script(SEGMENT_PUT, segment_params(&current))
            .map_err(db_err)?;
        transaction.commit().map_err(db_err)?;
        Ok(Some(current))
    }
}

fn is_retryable_provider_failure(failure: &str) -> bool {
    failure.starts_with("provider summary failed:")
}
