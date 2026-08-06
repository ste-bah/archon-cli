//! [`BoardAccess`] for the three ways a process reaches the memory graph.
//!
//! The board is only useful if a second Archon process can read and write it,
//! and every process after the first reaches the graph over TCP because CozoDB
//! admits one writer. A direct-only board would silently be a private board.

use crate::board::{BoardAccess, BoardItem, BoardStatus, BoardUpdate, NewBoardItem};
use crate::client::MemoryClient;
use crate::graph::MemoryGraph;
use crate::types::MemoryError;

use super::MemoryAccess;
use super::client_impl::block_on_async;

// ── MemoryGraph impl ───────────────────────────────────────────

impl BoardAccess for MemoryGraph {
    fn create_board_item(&self, item: &NewBoardItem) -> Result<BoardItem, MemoryError> {
        MemoryGraph::create_board_item(self, item)
    }

    fn get_board_item(&self, id: &str) -> Result<BoardItem, MemoryError> {
        MemoryGraph::get_board_item(self, id)
    }

    fn list_board_items_by_run(
        &self,
        run_id: &str,
        statuses: &[BoardStatus],
    ) -> Result<Vec<BoardItem>, MemoryError> {
        MemoryGraph::list_board_items_by_run(self, run_id, statuses)
    }

    fn claim_board_item(&self, id: &str, agent_id: &str) -> Result<BoardUpdate, MemoryError> {
        MemoryGraph::claim_board_item(self, id, agent_id)
    }

    fn release_board_claim(&self, id: &str) -> Result<BoardUpdate, MemoryError> {
        MemoryGraph::release_board_claim(self, id)
    }

    fn set_board_item_status(
        &self,
        id: &str,
        from: BoardStatus,
        to: BoardStatus,
    ) -> Result<BoardUpdate, MemoryError> {
        MemoryGraph::set_board_item_status(self, id, from, to)
    }
}

// ── MemoryClient impl ──────────────────────────────────────────

fn status_names(statuses: &[BoardStatus]) -> Vec<String> {
    statuses.iter().map(BoardStatus::to_string).collect()
}

impl BoardAccess for MemoryClient {
    fn create_board_item(&self, item: &NewBoardItem) -> Result<BoardItem, MemoryError> {
        let item = serde_json::to_value(item)?;
        let result =
            block_on_async(self.call("create_board_item", serde_json::json!({ "item": item })))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn get_board_item(&self, id: &str) -> Result<BoardItem, MemoryError> {
        let result = block_on_async(self.call("get_board_item", serde_json::json!({ "id": id })))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn list_board_items_by_run(
        &self,
        run_id: &str,
        statuses: &[BoardStatus],
    ) -> Result<Vec<BoardItem>, MemoryError> {
        let result = block_on_async(self.call(
            "list_board_items_by_run",
            serde_json::json!({ "run_id": run_id, "statuses": status_names(statuses) }),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn claim_board_item(&self, id: &str, agent_id: &str) -> Result<BoardUpdate, MemoryError> {
        let result = block_on_async(self.call(
            "claim_board_item",
            serde_json::json!({ "id": id, "agent_id": agent_id }),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn release_board_claim(&self, id: &str) -> Result<BoardUpdate, MemoryError> {
        let result =
            block_on_async(self.call("release_board_claim", serde_json::json!({ "id": id })))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }

    fn set_board_item_status(
        &self,
        id: &str,
        from: BoardStatus,
        to: BoardStatus,
    ) -> Result<BoardUpdate, MemoryError> {
        let result = block_on_async(self.call(
            "set_board_item_status",
            serde_json::json!({
                "id": id,
                "from": from.to_string(),
                "to": to.to_string(),
            }),
        ))?;
        serde_json::from_value(result).map_err(MemoryError::from)
    }
}

// ── MemoryAccess impl ──────────────────────────────────────────

impl BoardAccess for MemoryAccess {
    fn create_board_item(&self, item: &NewBoardItem) -> Result<BoardItem, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.create_board_item(item),
            Self::Remote(client) => client.create_board_item(item),
        }
    }

    fn get_board_item(&self, id: &str) -> Result<BoardItem, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.get_board_item(id),
            Self::Remote(client) => client.get_board_item(id),
        }
    }

    fn list_board_items_by_run(
        &self,
        run_id: &str,
        statuses: &[BoardStatus],
    ) -> Result<Vec<BoardItem>, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.list_board_items_by_run(run_id, statuses),
            Self::Remote(client) => client.list_board_items_by_run(run_id, statuses),
        }
    }

    fn claim_board_item(&self, id: &str, agent_id: &str) -> Result<BoardUpdate, MemoryError> {
        match self {
            // Claims must resolve in the one process that owns the writer, or
            // the compare-and-set is only a compare-and-set per process.
            Self::Direct { graph, .. } => graph.claim_board_item(id, agent_id),
            Self::Remote(client) => client.claim_board_item(id, agent_id),
        }
    }

    fn release_board_claim(&self, id: &str) -> Result<BoardUpdate, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.release_board_claim(id),
            Self::Remote(client) => client.release_board_claim(id),
        }
    }

    fn set_board_item_status(
        &self,
        id: &str,
        from: BoardStatus,
        to: BoardStatus,
    ) -> Result<BoardUpdate, MemoryError> {
        match self {
            Self::Direct { graph, .. } => graph.set_board_item_status(id, from, to),
            Self::Remote(client) => client.set_board_item_status(id, from, to),
        }
    }
}
