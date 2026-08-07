//! `GET /api/board/runs/{run_id}/activity` — one run's recent transitions.
//!
//! A sibling of `board.rs` rather than more of it, and the split is by question
//! rather than by size: everything there answers "what is on the board", and
//! this answers "what has been happening to it". The two are read together but
//! polled at different costs — the activity read is capped and ordered, the
//! board reads are not — so keeping them apart stops one growing the other's
//! shape.
//!
//! WHY THIS IS NOT `board_item_history` CALLED N TIMES. The dashboard shows a
//! run-wide feed, and per item that is one query per row on every poll. The
//! `board_item_events:by_run` index exists precisely so the run-scoped question
//! is a prefix read; asking it item by item would scan the same rows repeatedly
//! and then need the merge and the cap reimplemented in the client.

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use archon_memory::board::RUN_ACTIVITY_LIMIT;

use super::board::{WebBoardEvent, blocking, store_error, to_event};
use super::{AppState, check_auth, live::now_ms};

/// One run's recent transitions, newest first and bounded.
///
/// `limit` and `truncated` are on the wire because the cap is not a detail a
/// reader can infer: a feed showing exactly `limit` rows is indistinguishable
/// from a run that happened to have that many, and a UI that cannot tell the
/// difference will present a truncated history as a complete one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebBoardActivity {
    pub run_id: String,
    /// Newest first — the opposite of `WebBoardHistory`, which is one item's
    /// ladder read from the bottom. A feed is read from the top.
    pub events: Vec<WebBoardEvent>,
    /// The server-side cap, never exceeded by `events`.
    pub limit: u32,
    /// `true` when older transitions exist beyond `events`.
    pub truncated: bool,
    /// `false` when there is no memory database yet — a different answer from a
    /// database whose board has no history, and read differently in the UI.
    pub store_available: bool,
    pub observed_at_ms: u128,
}

pub(crate) async fn activity_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(run_id): Path<String>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let limit = RUN_ACTIVITY_LIMIT as u32;
    let board = match state.board.resolve(&state.paths).await {
        Ok(Some(board)) => board,
        Ok(None) => {
            return (
                StatusCode::OK,
                Json(WebBoardActivity {
                    run_id,
                    events: Vec::new(),
                    limit,
                    truncated: false,
                    store_available: false,
                    observed_at_ms: now_ms(),
                }),
            )
                .into_response();
        }
        Err(error) => return store_error(error),
    };
    let requested = run_id.clone();
    match blocking(board, move |board| board.board_run_activity(&requested)).await {
        Ok(events) => (
            StatusCode::OK,
            Json(WebBoardActivity {
                run_id,
                // The store truncates, so a full page is the only signal that
                // there was more. It over-reports by one run in the exact case
                // where the history is `limit` long and complete, which is the
                // harmless direction: claiming there may be more.
                truncated: events.len() >= RUN_ACTIVITY_LIMIT,
                events: events.iter().map(to_event).collect(),
                limit,
                store_available: true,
                observed_at_ms: now_ms(),
            }),
        )
            .into_response(),
        Err(error) => store_error(error),
    }
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    // `WebBoardEvent` is declared by `board::generated_typescript` and referenced
    // here; it must not be exported twice or the checked-in web.ts declares the
    // same name in one module and fails `npm run typecheck`.
    format!("export {}\n", WebBoardActivity::decl(&cfg))
}
