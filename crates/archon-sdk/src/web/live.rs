use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebLiveEvent {
    pub cursor: u64,
    pub event_type: String,
    pub summary: String,
    pub created_at_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebLiveSnapshot {
    pub events: Vec<WebLiveEvent>,
    pub next_cursor: u64,
    pub compacted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct WebLiveCursorExpired {
    pub cursor_expired: bool,
    pub oldest_available_cursor: u64,
    pub recovery: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveSnapshotQuery {
    after: Option<u64>,
}

#[derive(Clone)]
pub struct WebLiveManager {
    inner: Arc<Mutex<LiveBuffer>>,
}

#[derive(Debug)]
struct LiveBuffer {
    events: VecDeque<WebLiveEvent>,
    next_cursor: u64,
    max_events: usize,
    compacted: bool,
}

impl WebLiveManager {
    pub fn new(max_events: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(LiveBuffer {
                events: VecDeque::new(),
                next_cursor: 1,
                max_events,
                compacted: false,
            })),
        }
    }

    pub fn record(&self, event_type: impl Into<String>, summary: impl Into<String>) -> u64 {
        let mut inner = self.inner.lock().expect("live buffer mutex poisoned");
        let cursor = inner.next_cursor;
        inner.next_cursor += 1;
        if inner.events.len() >= inner.max_events {
            inner.events.pop_front();
            inner.compacted = true;
        }
        inner.events.push_back(WebLiveEvent {
            cursor,
            event_type: event_type.into(),
            summary: summary.into(),
            created_at_ms: now_ms(),
        });
        cursor
    }

    pub fn snapshot(&self, after: Option<u64>) -> Result<WebLiveSnapshot, WebLiveCursorExpired> {
        let inner = self.inner.lock().expect("live buffer mutex poisoned");
        let oldest = inner
            .events
            .front()
            .map(|event| event.cursor)
            .unwrap_or(inner.next_cursor);
        if let Some(cursor) = after
            && inner.compacted
            && cursor < oldest.saturating_sub(1)
        {
            return Err(WebLiveCursorExpired {
                cursor_expired: true,
                oldest_available_cursor: oldest,
                recovery: "refetch full snapshot".to_string(),
            });
        }

        let events = inner
            .events
            .iter()
            .filter(|event| after.is_none_or(|cursor| event.cursor > cursor))
            .cloned()
            .collect();
        Ok(WebLiveSnapshot {
            events,
            next_cursor: inner.next_cursor,
            compacted: inner.compacted,
        })
    }
}

pub(crate) async fn snapshot_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LiveSnapshotQuery>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    match state.live.snapshot(query.after) {
        Ok(snapshot) => (StatusCode::OK, Json(snapshot)).into_response(),
        Err(expired) => (StatusCode::CONFLICT, Json(expired)).into_response(),
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

pub fn generated_typescript() -> String {
    let cfg = TsConfig::default().with_large_int("number");
    [
        exported(WebLiveEvent::decl(&cfg)),
        exported(WebLiveSnapshot::decl(&cfg)),
        exported(WebLiveCursorExpired::decl(&cfg)),
    ]
    .join("\n\n")
        + "\n"
}

fn exported(decl: String) -> String {
    format!("export {decl}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_filters_by_cursor() {
        let live = WebLiveManager::new(8);
        let first = live.record("one", "first");
        live.record("two", "second");
        let snapshot = live.snapshot(Some(first)).unwrap();
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(snapshot.events[0].event_type, "two");
    }

    #[test]
    fn compacted_cursor_returns_expired() {
        let live = WebLiveManager::new(1);
        live.record("one", "first");
        live.record("two", "second");
        let expired = live.snapshot(Some(0)).unwrap_err();
        assert!(expired.cursor_expired);
    }
}
