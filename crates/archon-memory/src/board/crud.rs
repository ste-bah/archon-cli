use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use cozo::{DataValue, ScriptMutability};
use uuid::Uuid;

use super::rows::{BOARD_COLUMNS, BOARD_PUT, row_values_to_item, row_values_to_update};
use super::{BoardItem, BoardRunSummary, BoardStatus, NewBoardItem};
use crate::graph::MemoryGraph;
use crate::graph::helpers::run_mutable;
use crate::types::MemoryError;

pub(super) fn db_err(error: impl std::fmt::Display) -> MemoryError {
    MemoryError::Database(error.to_string())
}

impl MemoryGraph {
    /// Raise a board item and return the stored row.
    ///
    /// Empty `evidence` is a hard error, not a warning. An item without file
    /// references cannot be acted on by whoever picks it up — they would have to
    /// rediscover the finding from scratch, which is the failure the board
    /// exists to prevent. Rejecting at the write is the only place the rule can
    /// hold, because by the time a claimant reads the row the agent that knew
    /// the references is gone.
    ///
    /// An id already on the board is also rejected rather than overwritten:
    /// `:put` is last-writer-wins, and a colliding write would silently destroy
    /// another agent's item along with its round history.
    pub fn create_board_item(&self, item: &NewBoardItem) -> Result<BoardItem, MemoryError> {
        if item.evidence.trim().is_empty() {
            return Err(MemoryError::Database(
                "a board item needs evidence: file:line references and what was observed"
                    .to_string(),
            ));
        }
        if item.run_id.trim().is_empty() {
            return Err(MemoryError::Database(
                "a board item needs a run_id; the drain gate is defined over that partition"
                    .to_string(),
            ));
        }

        let id = item
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let now = Utc::now().to_rfc3339();
        let params = BTreeMap::from([
            ("id".to_string(), DataValue::from(id.as_str())),
            ("run_id".to_string(), DataValue::from(item.run_id.as_str())),
            ("kind".to_string(), DataValue::from(item.kind.to_string())),
            (
                "status".to_string(),
                DataValue::from(BoardStatus::Open.to_string()),
            ),
            ("title".to_string(), DataValue::from(item.title.as_str())),
            (
                "evidence".to_string(),
                DataValue::from(item.evidence.as_str()),
            ),
            (
                "acceptance".to_string(),
                DataValue::from(item.acceptance.as_str()),
            ),
            (
                "raised_by".to_string(),
                DataValue::from(item.raised_by.as_str()),
            ),
            ("claimed_by".to_string(), DataValue::Null),
            ("round".to_string(), DataValue::from(0i64)),
            ("created_at".to_string(), DataValue::from(now.as_str())),
            ("updated_at".to_string(), DataValue::from(now.as_str())),
        ]);

        let script = format!(
            "{{
                ?[{BOARD_COLUMNS}, applied] := *board_items{{{BOARD_COLUMNS}}},
                    id = $id, applied = false
            }} as _existing
            %if _existing
            %then %return _existing
            %end
            {{
                ?[{BOARD_COLUMNS}] <- [[$id, $run_id, $kind, $status, $title, $evidence,
                    $acceptance, $raised_by, $claimed_by, $round, $created_at, $updated_at]]
                {BOARD_PUT}
            }}
            {{
                ?[{BOARD_COLUMNS}, applied] := *board_items{{{BOARD_COLUMNS}}},
                    id = $id, applied = true
            }} as _created
            %return _created"
        );

        let result = run_mutable(&self.db, &script, params, "board: create item")?;
        let row = result
            .rows
            .first()
            .ok_or_else(|| MemoryError::Database("board create returned no row".to_string()))?;
        let update = row_values_to_update(row)?;
        if !update.applied {
            return Err(MemoryError::Database(format!(
                "board item {id} already exists; raised by {}",
                update.item.raised_by
            )));
        }
        Ok(update.item)
    }

    /// Read one board item by id.
    pub fn get_board_item(&self, id: &str) -> Result<BoardItem, MemoryError> {
        let params = BTreeMap::from([("id".to_string(), DataValue::from(id))]);
        let script = format!("?[{BOARD_COLUMNS}] := *board_items{{{BOARD_COLUMNS}}}, id = $id");
        let result = self
            .db
            .run_script(&script, params, ScriptMutability::Immutable)
            .map_err(db_err)?;
        let row = result
            .rows
            .first()
            .ok_or_else(|| MemoryError::NotFound(id.to_string()))?;
        self.with_decline_reason(row_values_to_item(row)?)
    }

    /// Items owned by `run_id`, oldest first; an empty `statuses` means all.
    ///
    /// Goes through the `board_items:by_run` index rather than filtering a scan.
    /// A board is polled — the drain gate reads it at every reduce, and every
    /// agent looking for work reads it again — so the per-read cost is paid
    /// constantly and must not grow with the size of the whole board.
    pub fn list_board_items_by_run(
        &self,
        run_id: &str,
        statuses: &[BoardStatus],
    ) -> Result<Vec<BoardItem>, MemoryError> {
        let mut params = BTreeMap::from([("run_id".to_string(), DataValue::from(run_id))]);
        let status_clause = if statuses.is_empty() {
            String::new()
        } else {
            params.insert(
                "statuses".to_string(),
                DataValue::List(
                    statuses
                        .iter()
                        .map(|status| DataValue::from(status.to_string()))
                        .collect(),
                ),
            );
            ", is_in(status, $statuses)".to_string()
        };
        let script = format!(
            "?[{BOARD_COLUMNS}] := *board_items:by_run{{run_id, id}},
                run_id = $run_id,
                *board_items{{{BOARD_COLUMNS}}}{status_clause}"
        );
        let result = self
            .db
            .run_script(&script, params, ScriptMutability::Immutable)
            .map_err(db_err)?;
        let mut items = result
            .rows
            .iter()
            .map(|row| row_values_to_item(row))
            .collect::<Result<Vec<_>, _>>()?;
        // Oldest first: a board is read as a queue of outstanding work, and the
        // item raised first is the one that has been waiting longest.
        items.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        self.with_decline_reasons_for_run(run_id, items)
    }

    /// Every run with items on the board, most recently touched first.
    ///
    /// Deliberately a scan and a fold in Rust, where every other read here goes
    /// through `board_items:by_run`. The index is keyed by `run_id`, so it can
    /// answer "which items are in this run" but not "which runs exist" — the
    /// distinct keys are exactly what an index lookup needs supplied. Nothing on
    /// the hot path calls this: the drain gate and the agents looking for work
    /// all arrive holding a `run_id` already, and this exists for the reader who
    /// does not. Paying a scan on a view that renders once per poll is the
    /// cheaper mistake than carrying a second relation that every board write
    /// would have to keep in step.
    ///
    /// `decline_reason` is not resolved here. A run summary is counts, and
    /// filling reasons in would mean the history query for every declined item
    /// in every run to populate a field this shape does not carry.
    pub fn list_board_runs(&self) -> Result<Vec<BoardRunSummary>, MemoryError> {
        let script = "?[run_id, status, updated_at] := *board_items{run_id, status, updated_at}";
        let result = self
            .db
            .run_script(script, BTreeMap::new(), ScriptMutability::Immutable)
            .map_err(db_err)?;

        let mut runs: BTreeMap<String, BoardRunSummary> = BTreeMap::new();
        for row in &result.rows {
            let run_id = row
                .first()
                .and_then(DataValue::get_str)
                .unwrap_or_default()
                .to_string();
            let status = row
                .get(1)
                .and_then(DataValue::get_str)
                .unwrap_or_default()
                .to_string();
            // An unparseable status is still an item that exists, and dropping
            // the row would understate the run. It is counted under whatever the
            // column holds rather than discarded.
            let updated_at = row
                .get(2)
                .and_then(DataValue::get_str)
                .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
                .map(|stamp| stamp.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);

            let summary = runs
                .entry(run_id.clone())
                .or_insert_with(|| BoardRunSummary {
                    run_id,
                    counts: BTreeMap::new(),
                    total: 0,
                    last_updated_at: updated_at,
                });
            *summary.counts.entry(status).or_insert(0) += 1;
            summary.total += 1;
            summary.last_updated_at = summary.last_updated_at.max(updated_at);
        }

        let mut runs: Vec<BoardRunSummary> = runs.into_values().collect();
        // Most recently touched first: a reader arriving without a run_id wants
        // the run something just happened in. `run_id` breaks ties so the order
        // is total — two runs can share a timestamp at second resolution, and a
        // list that reshuffles between polls reads as movement that did not
        // happen.
        runs.sort_by(|left, right| {
            right
                .last_updated_at
                .cmp(&left.last_updated_at)
                .then_with(|| left.run_id.cmp(&right.run_id))
        });
        Ok(runs)
    }
}
