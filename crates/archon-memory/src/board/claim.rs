use std::collections::BTreeMap;

use chrono::Utc;
use cozo::DataValue;

use super::rows::{BOARD_COLUMNS, BOARD_PUT, row_values_to_update};
use super::{BoardStatus, BoardUpdate};
use crate::graph::MemoryGraph;
use crate::graph::helpers::run_mutable;
use crate::types::MemoryError;

impl MemoryGraph {
    /// Compare-and-set over one board item.
    ///
    /// `guard_binding` names the columns the precondition reads and
    /// `guard_expr` is the precondition itself; `write_rule` is the rule that
    /// replaces the row, and it repeats the precondition. The repetition is
    /// deliberate: the `%if` decides whether the block runs at all, and the rule
    /// body decides which rows it touches, so a mistake in either one alone
    /// cannot produce a write that ignores the prior state.
    ///
    /// The whole script is one Cozo transaction, and `run_mutable` holds the
    /// `archon-cozo` write guard across it — the process mutex, the cross-process
    /// file lock, and the SQLITE_BUSY retry. `applied` therefore reports what
    /// this transaction did, not what a read a moment earlier suggested it might
    /// do. That distinction is the whole feature: two agents racing for one item
    /// both see an unclaimed row if they are allowed to look first.
    fn board_cas(
        &self,
        id: &str,
        guard_binding: &str,
        guard_expr: &str,
        write_rule: &str,
        mut params: BTreeMap<String, DataValue>,
        context: &str,
    ) -> Result<BoardUpdate, MemoryError> {
        params.insert("id".to_string(), DataValue::from(id));
        params.insert(
            "now".to_string(),
            DataValue::from(Utc::now().to_rfc3339().as_str()),
        );

        // The trailing unconditional block is reached only when `%return` in the
        // `%then` branch did not fire, so it needs no `%else` -- the same shape
        // `store_memory_with_id_outcome` uses.
        let script = format!(
            "{{
                ?[id] := *board_items{{{guard_binding}}}, id = $id, {guard_expr}
            }} as _eligible
            %if _eligible
            %then
                {{
                    {write_rule}
                    {BOARD_PUT}
                }}
                {{
                    ?[{BOARD_COLUMNS}, applied] := *board_items{{{BOARD_COLUMNS}}},
                        id = $id, applied = true
                }} as _applied
                %return _applied
            %end
            {{
                ?[{BOARD_COLUMNS}, applied] := *board_items{{{BOARD_COLUMNS}}},
                    id = $id, applied = false
            }} as _unchanged
            %return _unchanged"
        );

        let result = run_mutable(&self.db, &script, params, context)?;
        let row = result
            .rows
            .first()
            .ok_or_else(|| MemoryError::NotFound(id.to_string()))?;
        row_values_to_update(row)
    }

    /// Take ownership of an unclaimed item.
    ///
    /// `applied` is true for exactly one caller in a race. A loser gets
    /// `applied: false` alongside the authoritative row, so it can see who won
    /// rather than merely being refused.
    pub fn claim_board_item(&self, id: &str, agent_id: &str) -> Result<BoardUpdate, MemoryError> {
        if agent_id.trim().is_empty() {
            return Err(MemoryError::Database(
                "a claim needs an agent id; an anonymous claim has no liveness signal".to_string(),
            ));
        }
        let params = BTreeMap::from([
            ("agent_id".to_string(), DataValue::from(agent_id)),
            (
                "status".to_string(),
                DataValue::from(BoardStatus::Claimed.to_string()),
            ),
        ]);
        let write_rule = format!(
            "?[{BOARD_COLUMNS}] :=
                *board_items{{id, run_id, kind, title, evidence, acceptance, raised_by,
                    claimed_by: prior_claimed_by, round, created_at}},
                id = $id,
                is_null(prior_claimed_by),
                status = $status,
                claimed_by = $agent_id,
                updated_at = $now"
        );
        self.board_cas(
            id,
            "id, claimed_by",
            "is_null(claimed_by)",
            &write_rule,
            params,
            "board: claim item",
        )
    }

    /// Give an item back.
    ///
    /// `applied` is false when the item was not claimed, so a caller releasing
    /// twice learns the second call did nothing instead of assuming it undid
    /// someone else's claim. A status of `claimed` reverts to `open`; anything
    /// further along the lifecycle is left alone, because releasing the agent is
    /// not the same as retracting the work it already recorded.
    pub fn release_board_claim(&self, id: &str) -> Result<BoardUpdate, MemoryError> {
        let params = BTreeMap::from([
            (
                "claimed_status".to_string(),
                DataValue::from(BoardStatus::Claimed.to_string()),
            ),
            (
                "open_status".to_string(),
                DataValue::from(BoardStatus::Open.to_string()),
            ),
        ]);
        let write_rule = format!(
            "?[{BOARD_COLUMNS}] :=
                *board_items{{id, run_id, kind, status: prior_status, title, evidence,
                    acceptance, raised_by, claimed_by: prior_claimed_by, round, created_at}},
                id = $id,
                !is_null(prior_claimed_by),
                claimed_by = null,
                status = if(prior_status == $claimed_status, $open_status, prior_status),
                updated_at = $now"
        );
        self.board_cas(
            id,
            "id, claimed_by",
            "!is_null(claimed_by)",
            &write_rule,
            params,
            "board: release claim",
        )
    }

    /// Move an item between statuses, conditional on `from` still holding.
    ///
    /// A transition is a compare-and-set for the same reason a claim is: the
    /// reviewer marking an item `resolved` and the parent marking it
    /// `escalated` are separate agents acting on separate reads, and an
    /// unconditional write would let the later one erase a verdict it never
    /// saw.
    pub fn set_board_item_status(
        &self,
        id: &str,
        from: BoardStatus,
        to: BoardStatus,
    ) -> Result<BoardUpdate, MemoryError> {
        let params = BTreeMap::from([
            ("from".to_string(), DataValue::from(from.to_string())),
            ("to".to_string(), DataValue::from(to.to_string())),
        ]);
        let write_rule = format!(
            "?[{BOARD_COLUMNS}] :=
                *board_items{{id, run_id, kind, status: prior_status, title, evidence,
                    acceptance, raised_by, claimed_by, round, created_at}},
                id = $id,
                prior_status = $from,
                status = $to,
                updated_at = $now"
        );
        self.board_cas(
            id,
            "id, status",
            "status = $from",
            &write_rule,
            params,
            "board: set item status",
        )
    }
}
