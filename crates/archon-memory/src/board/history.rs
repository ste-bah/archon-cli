//! The board's transition history, and the decline reason derived from it.
//!
//! `board_item_events` is append-only and per item. Nothing rewrites a row: a
//! correction is another transition, which is what makes the relation readable
//! as a ladder rather than as a mutable summary of one.
//!
//! WHY CLAIMS AND RELEASES ARE NOT RECORDED HERE
//!
//! A claim is ownership churn, not a decision about the work: an agent that
//! takes an item, dies, and has its lease swept has changed nothing about the
//! item's standing, and a history full of that is a history nobody reads. Who
//! holds an item now is already on the row, in `claimed_by`. What this relation
//! carries is the transitions that a later reader has to be able to justify —
//! and those are exactly the ones whose caller had to name a `from` and a `to`.

use std::collections::BTreeMap;
use std::collections::HashMap;

use chrono::{DateTime, Utc};
use cozo::{DataValue, ScriptMutability};

use super::crud::db_err;
use super::{BoardEvent, BoardItem, BoardStatus};
use crate::graph::MemoryGraph;
use crate::types::MemoryError;

/// The `board_item_events` columns, in the one order every script here uses.
///
/// Positional like [`super::rows::BOARD_COLUMNS`] and for the same reason: the
/// decoder indexes by position, and `from_status` landing in `to_status` would
/// read as a plausible transition in the wrong direction.
const EVENT_COLUMNS: &str = "item_id, seq, at, run_id, from_status, to_status, round, actor, note";

/// The `:put` header matching [`EVENT_COLUMNS`].
const EVENT_PUT: &str = ":put board_item_events { item_id, seq => at, run_id, from_status, \
     to_status, round, actor, note }";

/// The chunk that appends one transition, for splicing into a CAS `%then` block.
///
/// It runs inside the same Cozo transaction as the write it describes, after
/// it, so `round` and `actor` are read from the row the transition produced. An
/// event written in a second transaction could be lost while the status change
/// survived, and a decline whose reason went missing is precisely the state the
/// drain gate exists to refuse.
///
/// `seq` is allocated here rather than by the caller. `seen` is seeded with `-1`
/// by its second rule so the aggregation below it always has a row to fold: an
/// item's first transition gets `0` without the script needing to know it is the
/// first.
pub(super) fn transition_event_chunk() -> String {
    format!(
        "{{
            seen[s] := *board_item_events{{item_id: $id, seq: s}}
            seen[s] := s = -1
            next[max(s)] := seen[s]
            ?[{EVENT_COLUMNS}] :=
                next[prior],
                *board_items{{id, run_id, round, claimed_by: actor}},
                id = $id,
                item_id = $id,
                seq = prior + 1,
                at = $now,
                from_status = $from,
                to_status = $to,
                note = $note
            {EVENT_PUT}
        }}"
    )
}

fn str_at(row: &[DataValue], index: usize) -> String {
    row.get(index)
        .and_then(DataValue::get_str)
        .unwrap_or_default()
        .to_string()
}

fn row_values_to_event(row: &[DataValue]) -> Result<BoardEvent, MemoryError> {
    let status_at = |index: usize| {
        let raw = str_at(row, index);
        BoardStatus::from_str_opt(&raw)
            .ok_or_else(|| MemoryError::InvalidType(format!("board event status: {raw}")))
    };
    Ok(BoardEvent {
        item_id: str_at(row, 0),
        seq: row.get(1).and_then(DataValue::get_int).unwrap_or(0).max(0) as u32,
        at: DateTime::parse_from_rfc3339(&str_at(row, 2))
            .map(|stamp| stamp.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        run_id: str_at(row, 3),
        from_status: status_at(4)?,
        to_status: status_at(5)?,
        round: row.get(6).and_then(DataValue::get_int).unwrap_or(0).max(0) as u32,
        // Null is "nobody held it", and must survive as `None` rather than
        // becoming `Some("")` — an event attributed to an agent named nothing
        // is worse than one honestly attributed to no one.
        actor: row.get(7).and_then(DataValue::get_str).map(String::from),
        note: str_at(row, 8),
    })
}

impl MemoryGraph {
    /// Every recorded transition for one item, oldest first.
    pub fn board_item_history(&self, id: &str) -> Result<Vec<BoardEvent>, MemoryError> {
        let params = BTreeMap::from([("id".to_string(), DataValue::from(id))]);
        let script =
            format!("?[{EVENT_COLUMNS}] := *board_item_events{{{EVENT_COLUMNS}}}, item_id = $id");
        let result = self
            .db
            .run_script(&script, params, ScriptMutability::Immutable)
            .map_err(db_err)?;
        let mut events = result
            .rows
            .iter()
            .map(|row| row_values_to_event(row))
            .collect::<Result<Vec<_>, _>>()?;
        events.sort_by_key(|event| event.seq);
        Ok(events)
    }

    /// Fill in `decline_reason` on an item that has one.
    ///
    /// A second query, and only for a declined item: every other status has no
    /// reason to read, and the board is polled often enough that a query per row
    /// per read would be paid constantly for an answer that is almost always
    /// absent.
    pub(super) fn with_decline_reason(
        &self,
        mut item: BoardItem,
    ) -> Result<BoardItem, MemoryError> {
        if item.status == BoardStatus::Declined {
            item.decline_reason = self
                .decline_reasons(
                    "?[item_id, seq, note] := *board_item_events{item_id, seq, to_status, note}, \
                     item_id = $key, to_status = $declined",
                    &item.id,
                )?
                .remove(&item.id);
        }
        Ok(item)
    }

    /// Fill in `decline_reason` across a run's items in one extra query.
    pub(super) fn with_decline_reasons_for_run(
        &self,
        run_id: &str,
        mut items: Vec<BoardItem>,
    ) -> Result<Vec<BoardItem>, MemoryError> {
        if !items
            .iter()
            .any(|item| item.status == BoardStatus::Declined)
        {
            return Ok(items);
        }
        let mut reasons = self.decline_reasons(
            "?[item_id, seq, note] := *board_item_events:by_run{run_id, item_id, seq}, \
             run_id = $key, *board_item_events{item_id, seq, to_status, note}, \
             to_status = $declined",
            run_id,
        )?;
        for item in &mut items {
            if item.status == BoardStatus::Declined {
                item.decline_reason = reasons.remove(&item.id);
            }
        }
        Ok(items)
    }

    /// Latest non-empty decline note per item, for whatever `script` selects.
    ///
    /// "Latest" is resolved here rather than in Datalog: the rows for one key
    /// are a handful, and an aggregation that had to be repeated correctly in
    /// two scripts is two chances to write `min`. An item can be declined more
    /// than once — declined, reopened by a parent, declined again — and only the
    /// standing reason is the one the item is closed on.
    fn decline_reasons(
        &self,
        script: &str,
        key: &str,
    ) -> Result<HashMap<String, String>, MemoryError> {
        let params = BTreeMap::from([
            ("key".to_string(), DataValue::from(key)),
            (
                "declined".to_string(),
                DataValue::from(BoardStatus::Declined.to_string()),
            ),
        ]);
        let result = self
            .db
            .run_script(script, params, ScriptMutability::Immutable)
            .map_err(db_err)?;
        let mut latest: HashMap<String, (i64, String)> = HashMap::new();
        for row in &result.rows {
            let note = str_at(row, 2);
            if note.trim().is_empty() {
                continue;
            }
            let seq = row.get(1).and_then(DataValue::get_int).unwrap_or(0);
            let entry = latest
                .entry(str_at(row, 0))
                .or_insert((i64::MIN, String::new()));
            if seq >= entry.0 {
                *entry = (seq, note);
            }
        }
        Ok(latest
            .into_iter()
            .map(|(id, (_, note))| (id, note))
            .collect())
    }
}
