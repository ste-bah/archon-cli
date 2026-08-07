//! `GET /api/board/*` — the agent task board, read-only.
//!
//! UNLIKE `/api/agents/live`, THIS IS NOT GATED ON ATTACHED MODE. That endpoint
//! reads `BACKGROUND_AGENTS` and `TASK_MANAGER`, which own `JoinHandle`s and
//! cannot cross a process boundary, so it is meaningful only inside the session
//! it reports on. The board is not a registry: it lives in the memory database,
//! which is a file, and any process that can open that file sees the same rows.
//! A standalone `archon web` therefore shows the real board, not an empty one.
//!
//! Reaching it is what this module has to arrange, and it cannot do what its
//! neighbours do. `inspect.rs` reuses `WebRuntimeHandles::memory`, the handle the
//! host session already has open, and falls back to opening the file only when
//! there is no handle — the right shape, and unavailable here: that handle is an
//! `Arc<dyn MemoryTrait>`, the board is deliberately NOT on `MemoryTrait` (see
//! `archon_memory::board`, which keeps `BoardAccess` separate so seventeen mock
//! implementations do not grow board methods), and no `BoardAccess` can be
//! recovered from a `MemoryTrait` object.
//!
//! WHAT IT MUST NOT DO INSTEAD IS OPEN THE DATABASE ITSELF. CozoDB admits one
//! writer, and in attached mode the host session already holds it on this very
//! file, so a direct `MemoryGraph::open` here is a second writer against a locked
//! database. `open_memory_with_db_path` is the substitute for the handle this
//! module cannot borrow: the singleton election reads `memory.port`, connects as
//! a client when a server answers, and only otherwise takes `memory.lock` and
//! opens the graph. `MemoryAccess` implements `BoardAccess`, which is what makes
//! that work — the server is `Direct` when it is the only process and `Remote`
//! over TCP when the TUI owns the writer, decided by the same code path as every
//! other entry point in Archon.
//!
//! Like the agent panel, this is a snapshot rather than an append-only log — the
//! statuses of items that already exist change in place — so it is polled by the
//! client. Streaming it would mean diffing or resending the board on every
//! transition.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use archon_memory::access::MemoryAccess;
use archon_memory::board::{BoardAccess, BoardEvent, BoardItem, BoardRunSummary, BoardStatus};
use archon_memory::open_memory_with_db_path;
use archon_memory::types::MemoryError;

use super::{AppState, WebRuntimePaths, check_auth, live::now_ms};

/// The board reader, elected on first use and shared afterwards.
///
/// A `OnceCell` and not a `OnceLock`: the election is async, and it is the
/// async-aware cell whose `get_or_try_init` stores only on success. A failed
/// election must not be cached — the memory server can be down when the first
/// request arrives and up a minute later, and a cell that recorded the failure
/// would keep reporting an unreachable board for the life of the process. A
/// success is cached for exactly the opposite reason: re-running the election
/// per request would re-read the port file and reconnect on every poll, and in
/// the `Direct` branch would try to take a lock this process already holds.
///
/// It holds the elected `MemoryAccess` rather than an `Arc<dyn BoardAccess>`
/// because which arm was elected is the thing worth asserting: a second direct
/// open of a held database does not hang and does not error — it silently
/// succeeds and even reads correctly — so no behavioural test can tell the two
/// apart. The mode is the only observable difference, and `elected` exists so a
/// test can pin it.
#[derive(Clone, Default)]
pub struct WebBoardStore {
    opened: Arc<tokio::sync::OnceCell<Arc<MemoryAccess>>>,
}

impl WebBoardStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The board reader, or `None` when there is no database yet.
    ///
    /// `None` is a real answer rather than a failure, and it is checked before
    /// the election because `open_memory_with_db_path` creates what it cannot
    /// find. Looking at a dashboard must not be what brings a memory database
    /// into existence.
    pub(super) async fn resolve(
        &self,
        paths: &WebRuntimePaths,
    ) -> Result<Option<Arc<dyn BoardAccess>>, String> {
        if let Some(access) = self.opened.get() {
            return Ok(Some(Arc::clone(access) as Arc<dyn BoardAccess>));
        }
        if !paths.memory_db.exists() {
            return Ok(None);
        }
        let access = self
            .opened
            .get_or_try_init(|| async {
                // The election coordinates on `memory.port` and `memory.lock` in
                // the data directory, not on the database file. That directory
                // is the database's parent in every case `resolve_memory_paths`
                // produces — including a configured path, where it derives the
                // data dir from the `.db` file's parent — so taking it from here
                // lands on the same files the session is coordinating through.
                //
                // A bare relative filename has `Some("")` as its parent, not
                // `None`, and coordinating in "" would put the port file
                // somewhere neither process agrees on.
                let data_dir = paths
                    .memory_db
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(&paths.archon_data);
                let access = open_memory_with_db_path(data_dir, &paths.memory_db)
                    .await
                    .map_err(|error| format!("board: could not reach memory: {error}"))?;
                Ok::<Arc<MemoryAccess>, String>(Arc::new(access))
            })
            .await?;
        Ok(Some(Arc::clone(access) as Arc<dyn BoardAccess>))
    }

    /// Which arm the election returned, once it has run.
    #[cfg(test)]
    fn elected(&self) -> Option<Arc<MemoryAccess>> {
        self.opened.get().map(Arc::clone)
    }
}

/// One run with items on the board.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebBoardRun {
    pub run_id: String,
    pub total: u32,
    /// Per-status counts, only for statuses the run actually has items in.
    pub counts: Vec<WebBoardStatusCount>,
    /// RFC 3339. The newest `updated_at` in the run, and the sort key.
    pub last_updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebBoardStatusCount {
    pub status: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebBoardRunList {
    /// Most recently touched first.
    pub runs: Vec<WebBoardRun>,
    /// `false` when there is no memory database yet, which is a different
    /// answer from a database holding an empty board and reads differently in
    /// the UI.
    pub store_available: bool,
    pub observed_at_ms: u128,
}

/// One board item, projected for the dashboard.
///
/// Mirrors `archon_memory::board::BoardItem` field for field rather than
/// re-exporting it: the storage type is not a `ts_rs::TS` type and making it one
/// would put a web concern in the crate the drain gate depends on. The
/// timestamps become strings here because that is what crosses JSON anyway.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebBoardItem {
    pub id: String,
    pub run_id: String,
    /// `issue` or `note`.
    pub kind: String,
    pub status: String,
    pub title: String,
    /// File references and what was observed. Required at the store, so never
    /// empty on an item that exists.
    pub evidence: String,
    /// What "done" means for this item.
    pub acceptance: String,
    pub raised_by: String,
    /// The agent currently holding the item, `null` when unclaimed.
    pub claimed_by: Option<String>,
    /// Attempt counter, 0-based.
    pub round: u32,
    pub created_at: String,
    pub updated_at: String,
    /// Why the item was declined. Non-null only on a declined item, and the
    /// store refuses to record a decline without one.
    pub decline_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebBoardItems {
    pub run_id: String,
    /// Oldest first — the board is a queue, and the item raised first has been
    /// waiting longest.
    pub items: Vec<WebBoardItem>,
    /// The statuses that were filtered on, empty when all were requested.
    pub statuses: Vec<String>,
    pub store_available: bool,
    pub observed_at_ms: u128,
}

/// One recorded transition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebBoardEvent {
    pub item_id: String,
    /// Per-item, 0-based.
    pub seq: u32,
    pub at: String,
    pub from_status: String,
    pub to_status: String,
    pub round: u32,
    /// Who held the item at the time, `null` when nobody did.
    pub actor: Option<String>,
    /// What the transition recorded. Required for a decline, empty otherwise.
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebBoardHistory {
    pub item_id: String,
    /// Oldest first. Empty for an item that has never transitioned — claims and
    /// releases are not recorded, only decisions about the work.
    pub events: Vec<WebBoardEvent>,
    pub store_available: bool,
}

/// `?status=open,claimed`. Absent means every status.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WebBoardItemQuery {
    #[serde(default)]
    pub status: Option<String>,
}

pub(crate) async fn runs_handler(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let board = match state.board.resolve(&state.paths).await {
        Ok(Some(board)) => board,
        Ok(None) => {
            return (
                StatusCode::OK,
                Json(WebBoardRunList {
                    runs: Vec::new(),
                    store_available: false,
                    observed_at_ms: now_ms(),
                }),
            )
                .into_response();
        }
        Err(error) => return store_error(error),
    };
    match blocking(board, |board| board.list_board_runs()).await {
        Ok(runs) => (
            StatusCode::OK,
            Json(WebBoardRunList {
                runs: runs.iter().map(to_run).collect(),
                store_available: true,
                observed_at_ms: now_ms(),
            }),
        )
            .into_response(),
        Err(error) => store_error(error),
    }
}

pub(crate) async fn items_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
    Query(query): Query<WebBoardItemQuery>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let statuses = match parse_statuses(query.status.as_deref()) {
        Ok(statuses) => statuses,
        // A typo in a status name must not read as "no items in that status" —
        // an empty board is what this view reports when there is nothing to do.
        Err(unknown) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("board: unknown status filter: {unknown}"),
            )
                .into_response();
        }
    };
    let names: Vec<String> = statuses.iter().map(BoardStatus::to_string).collect();
    let board = match state.board.resolve(&state.paths).await {
        Ok(Some(board)) => board,
        Ok(None) => {
            return (
                StatusCode::OK,
                Json(WebBoardItems {
                    run_id,
                    items: Vec::new(),
                    statuses: names,
                    store_available: false,
                    observed_at_ms: now_ms(),
                }),
            )
                .into_response();
        }
        Err(error) => return store_error(error),
    };
    let requested = run_id.clone();
    match blocking(board, move |board| {
        board.list_board_items_by_run(&requested, &statuses)
    })
    .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(WebBoardItems {
                run_id,
                items: items.iter().map(to_item).collect(),
                statuses: names,
                store_available: true,
                observed_at_ms: now_ms(),
            }),
        )
            .into_response(),
        Err(error) => store_error(error),
    }
}

pub(crate) async fn history_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(item_id): Path<String>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let board = match state.board.resolve(&state.paths).await {
        Ok(Some(board)) => board,
        Ok(None) => {
            return (
                StatusCode::OK,
                Json(WebBoardHistory {
                    item_id,
                    events: Vec::new(),
                    store_available: false,
                }),
            )
                .into_response();
        }
        Err(error) => return store_error(error),
    };
    let requested = item_id.clone();
    match blocking(board, move |board| board.board_item_history(&requested)).await {
        Ok(events) => (
            StatusCode::OK,
            Json(WebBoardHistory {
                item_id,
                events: events.iter().map(to_event).collect(),
                store_available: true,
            }),
        )
            .into_response(),
        Err(error) => store_error(error),
    }
}

/// Run one board read off the runtime's worker threads.
///
/// Every `BoardAccess` method is synchronous, and the write path underneath goes
/// through the `archon-cozo` guard, whose SQLITE_BUSY retry loop parks the
/// calling thread with `thread::sleep`. Reads take the same guarded instance, so
/// calling one directly from a handler could stall the executor the same way the
/// memory server's dispatch would — which is why that also uses `spawn_blocking`.
pub(super) async fn blocking<T, F>(board: Arc<dyn BoardAccess>, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&dyn BoardAccess) -> Result<T, MemoryError> + Send + 'static,
{
    tokio::task::spawn_blocking(move || work(board.as_ref()).map_err(|error| error.to_string()))
        .await
        .map_err(|error| format!("board read did not complete: {error}"))?
}

pub(super) fn store_error(message: String) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, message).into_response()
}

/// Statuses named in the query string, or every status when none were.
///
/// Returns the offending name rather than a bool so the refusal can say which
/// one it did not recognise.
fn parse_statuses(raw: Option<&str>) -> Result<Vec<BoardStatus>, String> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    raw.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| BoardStatus::from_str_opt(name).ok_or_else(|| name.to_string()))
        .collect()
}

fn to_run(run: &BoardRunSummary) -> WebBoardRun {
    WebBoardRun {
        run_id: run.run_id.clone(),
        total: run.total,
        counts: run
            .counts
            .iter()
            .map(|(status, count)| WebBoardStatusCount {
                status: status.clone(),
                count: *count,
            })
            .collect(),
        last_updated_at: run.last_updated_at.to_rfc3339(),
    }
}

fn to_item(item: &BoardItem) -> WebBoardItem {
    WebBoardItem {
        id: item.id.clone(),
        run_id: item.run_id.clone(),
        kind: item.kind.to_string(),
        status: item.status.to_string(),
        title: item.title.clone(),
        evidence: item.evidence.clone(),
        acceptance: item.acceptance.clone(),
        raised_by: item.raised_by.clone(),
        claimed_by: item.claimed_by.clone(),
        round: item.round,
        created_at: item.created_at.to_rfc3339(),
        updated_at: item.updated_at.to_rfc3339(),
        decline_reason: item.decline_reason.clone(),
    }
}

pub(super) fn to_event(event: &BoardEvent) -> WebBoardEvent {
    WebBoardEvent {
        item_id: event.item_id.clone(),
        seq: event.seq,
        at: event.at.to_rfc3339(),
        from_status: event.from_status.to_string(),
        to_status: event.to_status.to_string(),
        round: event.round,
        actor: event.actor.clone(),
        note: event.note.clone(),
    }
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WebBoardRunList::decl(&cfg)),
        exported(WebBoardRun::decl(&cfg)),
        // Referenced by WebBoardRun. Unexported, the checked-in web.ts would
        // name a type it never declared and `npm run typecheck` would fail.
        exported(WebBoardStatusCount::decl(&cfg)),
        exported(WebBoardItems::decl(&cfg)),
        exported(WebBoardItem::decl(&cfg)),
        exported(WebBoardHistory::decl(&cfg)),
        exported(WebBoardEvent::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
#[path = "board_tests.rs"]
mod tests;
