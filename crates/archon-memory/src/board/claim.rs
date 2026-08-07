use std::collections::BTreeMap;

use chrono::Utc;
use cozo::DataValue;

use super::history::transition_event_chunk;
use super::rows::{BOARD_COLUMNS, BOARD_PUT, row_values_to_update};
use super::{BoardStatus, BoardUpdate};
use crate::graph::MemoryGraph;
use crate::graph::helpers::run_mutable;
use crate::types::MemoryError;

/// The Datalog a compare-and-set is assembled from.
///
/// Grouped rather than passed as four `&str`s, because four adjacent string
/// arguments is four chances to transpose two of them into a script that still
/// compiles and writes the wrong rows.
struct CasScript<'a> {
    /// The columns the precondition reads.
    guard_binding: &'a str,
    /// The precondition itself.
    guard_expr: &'a str,
    /// The rule that replaces the row. It REPEATS the precondition, and the
    /// repetition is deliberate: the `%if` decides whether the block runs at
    /// all, and the rule body decides which rows it touches, so a mistake in
    /// either one alone cannot produce a write that ignores the prior state.
    write_rule: &'a str,
    /// Spliced in behind the row write and inside the same `%then`, which is
    /// how a transition's history entry lands in the transaction that decided
    /// the transition. Empty for the writes that record no history — see
    /// [`super::history`] for which and why.
    after_write: &'a str,
}

impl MemoryGraph {
    /// Compare-and-set over one board item.
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
        cas: CasScript<'_>,
        mut params: BTreeMap<String, DataValue>,
        context: &str,
    ) -> Result<BoardUpdate, MemoryError> {
        let CasScript {
            guard_binding,
            guard_expr,
            write_rule,
            after_write,
        } = cas;
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
                {after_write}
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
        let mut update = row_values_to_update(row)?;
        update.item = self.with_decline_reason(update.item)?;
        Ok(update)
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
            CasScript {
                guard_binding: "id, claimed_by",
                guard_expr: "is_null(claimed_by)",
                write_rule: &write_rule,
                after_write: "",
            },
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
            CasScript {
                guard_binding: "id, claimed_by",
                guard_expr: "!is_null(claimed_by)",
                write_rule: &write_rule,
                after_write: "",
            },
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
    ///
    /// `Declined` is refused here. It is the one ending that closes an item on
    /// nothing but an assertion, so it has to carry a justification, and an
    /// `Option<&str>` on this method would make the justification something a
    /// caller can pass `None` for. [`Self::decline_board_item`] takes it as a
    /// `&str` instead, which is the difference between a rule and a check.
    pub fn set_board_item_status(
        &self,
        id: &str,
        from: BoardStatus,
        to: BoardStatus,
    ) -> Result<BoardUpdate, MemoryError> {
        if to == BoardStatus::Declined {
            return Err(MemoryError::Database(
                "declining a board item needs a reason: call decline_board_item(id, from, reason). \
                 The drain gate refuses a declined item with nothing recorded behind it"
                    .to_string(),
            ));
        }
        self.transition(id, from, to, "", "board: set item status")
    }

    /// Close an item as `declined`, recording why.
    ///
    /// An empty reason is refused at the write, the way empty evidence is on
    /// `create_board_item` and an anonymous claim is on `claim_board_item`: by
    /// the time the drain gate reads the row, the agent that knew the argument
    /// for declining is gone, and there is nowhere else the rule can hold.
    pub fn decline_board_item(
        &self,
        id: &str,
        from: BoardStatus,
        reason: &str,
    ) -> Result<BoardUpdate, MemoryError> {
        if reason.trim().is_empty() {
            return Err(MemoryError::Database(
                "a decline needs a reason: what was judged, and why the work should not happen"
                    .to_string(),
            ));
        }
        self.transition(
            id,
            from,
            BoardStatus::Declined,
            reason,
            "board: decline item",
        )
    }

    /// The status compare-and-set both public transitions share.
    fn transition(
        &self,
        id: &str,
        from: BoardStatus,
        to: BoardStatus,
        note: &str,
        context: &str,
    ) -> Result<BoardUpdate, MemoryError> {
        // A round is an ATTEMPT, so it advances exactly when work restarts:
        // leaving `gaps_remain` for a working status. Review sending an item
        // back is the only thing in the lifecycle that means "try again".
        //
        // Every other transition carries the count forward. Incrementing on
        // each move would make `round` a transition counter, which is what the
        // event history already is, and it would misreport a straight
        // open -> claimed -> resolved item as three attempts.
        //
        // Computed here rather than in Datalog because the rule is about the
        // pair, and a `%if` per transition would state it twice.
        let bump = i64::from(
            from == BoardStatus::GapsRemain
                && matches!(to, BoardStatus::Open | BoardStatus::Claimed),
        );
        let params = BTreeMap::from([
            ("from".to_string(), DataValue::from(from.to_string())),
            ("to".to_string(), DataValue::from(to.to_string())),
            ("note".to_string(), DataValue::from(note)),
            ("bump".to_string(), DataValue::from(bump)),
        ]);
        let write_rule = format!(
            "?[{BOARD_COLUMNS}] :=
                *board_items{{id, run_id, kind, status: prior_status, title, evidence,
                    acceptance, raised_by, claimed_by, round: prior_round, created_at}},
                id = $id,
                prior_status = $from,
                status = $to,
                round = prior_round + $bump,
                updated_at = $now"
        );
        self.board_cas(
            id,
            CasScript {
                guard_binding: "id, status",
                guard_expr: "status = $from",
                write_rule: &write_rule,
                after_write: &transition_event_chunk(),
            },
            params,
            context,
        )
    }
}
