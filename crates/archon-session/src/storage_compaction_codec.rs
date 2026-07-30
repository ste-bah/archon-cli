use std::collections::BTreeMap;

use cozo::DataValue;

use super::{CompactionLedgerRecord, CompactionSegment, CompactionSummaryStatus};
use crate::storage::{extract_f64, extract_i64, extract_str};

pub(super) const SEGMENT_PUT: &str = "?[id, session_id, start_index, end_index, status, summary, model, attribution, failure, input_tokens, output_tokens, cost, created_at, updated_at] <- [[$id, $session_id, $start_index, $end_index, $status, $summary, $model, $attribution, $failure, $input_tokens, $output_tokens, $cost, $created_at, $updated_at]]
:put compaction_segments {id => session_id, start_index, end_index, status, summary, model, attribution, failure, input_tokens, output_tokens, cost, created_at, updated_at}";

pub(super) fn segment_params(segment: &CompactionSegment) -> BTreeMap<String, DataValue> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(segment.id.as_str()));
    params.insert(
        "session_id".into(),
        DataValue::from(segment.session_id.as_str()),
    );
    params.insert(
        "start_index".into(),
        DataValue::from(segment.start_index as i64),
    );
    params.insert(
        "end_index".into(),
        DataValue::from(segment.end_index as i64),
    );
    params.insert(
        "status".into(),
        DataValue::from(segment.summary_status.as_str()),
    );
    params.insert(
        "summary".into(),
        DataValue::from(segment.summary.as_deref().unwrap_or("")),
    );
    params.insert(
        "model".into(),
        DataValue::from(segment.summary_model.as_deref().unwrap_or("")),
    );
    params.insert(
        "attribution".into(),
        DataValue::from(segment.summary_attribution.as_deref().unwrap_or("")),
    );
    params.insert(
        "failure".into(),
        DataValue::from(segment.summary_failure.as_deref().unwrap_or("")),
    );
    params.insert(
        "input_tokens".into(),
        DataValue::from(segment.summary_input_tokens.map(|v| v as i64).unwrap_or(-1)),
    );
    params.insert(
        "output_tokens".into(),
        DataValue::from(
            segment
                .summary_output_tokens
                .map(|v| v as i64)
                .unwrap_or(-1),
        ),
    );
    params.insert(
        "cost".into(),
        DataValue::from(segment.summary_cost.unwrap_or(-1.0)),
    );
    params.insert(
        "created_at".into(),
        DataValue::from(segment.created_at.as_str()),
    );
    params.insert(
        "updated_at".into(),
        DataValue::from(segment.updated_at.as_str()),
    );
    params
}

pub(super) fn segment_from_row(row: &[DataValue]) -> CompactionSegment {
    let optional_string = |index| {
        let value = extract_str(&row[index]);
        (!value.is_empty()).then_some(value)
    };
    let optional_u64 = |index| {
        let value = extract_i64(&row[index]);
        (value >= 0).then_some(value as u64)
    };
    let cost = extract_f64(&row[11]);
    CompactionSegment {
        id: extract_str(&row[0]),
        session_id: extract_str(&row[1]),
        start_index: extract_i64(&row[2]).max(0) as u64,
        end_index: extract_i64(&row[3]).max(0) as u64,
        summary_status: CompactionSummaryStatus::parse(&extract_str(&row[4])),
        summary: optional_string(5),
        summary_model: optional_string(6),
        summary_attribution: optional_string(7),
        summary_failure: optional_string(8),
        summary_input_tokens: optional_u64(9),
        summary_output_tokens: optional_u64(10),
        summary_cost: (cost >= 0.0).then_some(cost),
        created_at: extract_str(&row[12]),
        updated_at: extract_str(&row[13]),
    }
}

pub(super) fn ledger_params(record: &CompactionLedgerRecord) -> BTreeMap<String, DataValue> {
    let mut params = BTreeMap::new();
    params.insert("id".into(), DataValue::from(record.id.as_str()));
    params.insert(
        "session_id".into(),
        DataValue::from(record.session_id.as_str()),
    );
    params.insert("kind".into(), DataValue::from(record.kind.as_str()));
    params.insert("payload".into(), DataValue::from(record.payload.as_str()));
    params.insert(
        "start_index".into(),
        DataValue::from(record.source_start_index as i64),
    );
    params.insert(
        "end_index".into(),
        DataValue::from(record.source_end_index as i64),
    );
    params.insert(
        "created_at".into(),
        DataValue::from(record.created_at.as_str()),
    );
    params
}
