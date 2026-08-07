use std::{
    collections::VecDeque,
    convert::Infallible,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use serde::{Deserialize, Serialize};
use ts_rs::{Config as TsConfig, TS};

use super::{AppState, check_auth};

/// How often the stream re-reads the ring buffer.
///
/// The buffer is a plain `Mutex<VecDeque>` with no notification primitive, so
/// the stream has to poll it. One second matches the workflow event stream and
/// is well inside what a human reads as "live".
const STREAM_POLL: Duration = Duration::from_secs(1);

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

/// Poll state carried through the SSE stream.
struct StreamCursor {
    live: WebLiveManager,
    after: Option<u64>,
    interval: tokio::time::Interval,
    /// Set once the cursor-expired frame has been sent. The client must go
    /// back to `/api/live/snapshot` at that point, so there is nothing useful
    /// left to send on this connection.
    finished: bool,
}

/// `GET /api/live/stream` — the ring buffer as a server-sent event stream.
///
/// Each frame is a whole [`WebLiveSnapshot`], so the client gets `nextCursor`
/// and `compacted` on every frame instead of having to track them itself.
/// A cursor older than the compaction window produces one
/// [`WebLiveCursorExpired`] frame and then end-of-stream; the shapes are
/// distinguished client-side by the `cursorExpired` field.
pub(crate) async fn stream_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<LiveSnapshotQuery>,
) -> Response {
    if let Err(resp) = check_auth(&state, &headers) {
        return resp;
    }
    let cursor = StreamCursor {
        live: state.live.clone(),
        after: query.after,
        interval: tokio::time::interval(STREAM_POLL),
        finished: false,
    };
    let stream = futures_util::stream::unfold(cursor, |mut cursor| async move {
        if cursor.finished {
            return None;
        }
        loop {
            cursor.interval.tick().await;
            match cursor.live.snapshot(cursor.after) {
                // Nothing new: stay silent and let SSE keep-alive hold the
                // connection open rather than shipping an empty frame a second.
                Ok(snapshot) if snapshot.events.is_empty() => continue,
                Ok(snapshot) => {
                    cursor.after = Some(snapshot.next_cursor.saturating_sub(1));
                    return Some((Ok::<_, Infallible>(sse_event(&snapshot)), cursor));
                }
                Err(expired) => {
                    cursor.finished = true;
                    return Some((Ok::<_, Infallible>(sse_event(&expired)), cursor));
                }
            }
        }
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn sse_event<T: Serialize>(payload: &T) -> Event {
    Event::default()
        .event("live-snapshot")
        .json_data(payload)
        .unwrap_or_else(|_| {
            Event::default()
                .event("live-error")
                .data("serialization failed")
        })
}

pub(super) fn now_ms() -> u128 {
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
