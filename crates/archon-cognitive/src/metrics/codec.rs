//! Column codecs shared by the metric event and evaluation-window stores.

use chrono::{DateTime, Utc};
use cozo::DataValue;

use crate::CognitiveError;

pub(crate) fn str_col(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or("")
        .to_string()
}

pub(crate) fn int_col(row: &[DataValue], index: usize) -> i64 {
    row.get(index).and_then(DataValue::get_int).unwrap_or(0)
}

/// Nullable float column. `Null` and any non-numeric value both read back as
/// "absent", which is the same thing the writer meant by `None`.
pub(crate) fn opt_float_col(row: &[DataValue], index: usize) -> Option<f64> {
    row.get(index).and_then(DataValue::get_float)
}

pub(crate) fn time_col(row: &[DataValue], index: usize) -> Result<DateTime<Utc>, CognitiveError> {
    let raw = str_col(row, index);
    DateTime::parse_from_rfc3339(&raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| CognitiveError::Metric(format!("unreadable timestamp `{raw}`: {error}")))
}

pub(crate) fn opt_float_value(value: Option<f64>) -> DataValue {
    value.map_or(DataValue::Null, DataValue::from)
}

pub(crate) fn json_col<T: serde::de::DeserializeOwned + Default>(
    row: &[DataValue],
    index: usize,
) -> Result<T, CognitiveError> {
    let raw = str_col(row, index);
    if raw.is_empty() {
        return Ok(T::default());
    }
    Ok(serde_json::from_str(&raw)?)
}
