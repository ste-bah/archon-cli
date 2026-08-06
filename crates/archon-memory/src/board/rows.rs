use chrono::{DateTime, Utc};
use cozo::DataValue;

use super::{BoardItem, BoardItemKind, BoardStatus, BoardUpdate};
use crate::types::MemoryError;

/// The `board_items` columns, in the one order every script in this module
/// uses.
///
/// Written once because the row decoder indexes positionally: a query that
/// listed the columns in a different order would decode into the wrong fields
/// silently, and `evidence` landing in `acceptance` reads as plausible prose.
pub(crate) const BOARD_COLUMNS: &str = "id, run_id, kind, status, title, evidence, acceptance, \
     raised_by, claimed_by, round, created_at, updated_at";

/// The `:put` header matching [`BOARD_COLUMNS`].
pub(crate) const BOARD_PUT: &str = ":put board_items { id => run_id, kind, status, title, \
     evidence, acceptance, raised_by, claimed_by, round, created_at, updated_at }";

fn str_at(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or_default()
        .to_string()
}

fn timestamp_at(row: &[DataValue], index: usize) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(&str_at(row, index))
        .map(|stamp| stamp.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

/// Decode one `board_items` row laid out as [`BOARD_COLUMNS`].
pub(crate) fn row_values_to_item(row: &[DataValue]) -> Result<BoardItem, MemoryError> {
    let kind_raw = str_at(row, 2);
    let status_raw = str_at(row, 3);
    Ok(BoardItem {
        id: str_at(row, 0),
        run_id: str_at(row, 1),
        kind: BoardItemKind::from_str_opt(&kind_raw)
            .ok_or_else(|| MemoryError::InvalidType(format!("board item kind: {kind_raw}")))?,
        status: BoardStatus::from_str_opt(&status_raw)
            .ok_or_else(|| MemoryError::InvalidType(format!("board item status: {status_raw}")))?,
        title: str_at(row, 4),
        evidence: str_at(row, 5),
        acceptance: str_at(row, 6),
        raised_by: str_at(row, 7),
        // A null here is the unclaimed state and must survive as `None`; the
        // string fallback used elsewhere would turn it into `Some("")`, which
        // the claim CAS would then read as "already held by nobody-in-
        // particular" and refuse forever.
        claimed_by: row.get(8).and_then(DataValue::get_str).map(String::from),
        round: row.get(9).and_then(DataValue::get_int).unwrap_or(0).max(0) as u32,
        created_at: timestamp_at(row, 10),
        updated_at: timestamp_at(row, 11),
    })
}

/// Decode a row laid out as [`BOARD_COLUMNS`] followed by the `applied` flag.
pub(crate) fn row_values_to_update(row: &[DataValue]) -> Result<BoardUpdate, MemoryError> {
    let applied = row.get(12).and_then(DataValue::get_bool).ok_or_else(|| {
        MemoryError::Database("board write returned no in-transaction outcome".to_string())
    })?;
    Ok(BoardUpdate {
        applied,
        item: row_values_to_item(row)?,
    })
}
