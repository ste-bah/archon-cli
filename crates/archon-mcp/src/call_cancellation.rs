//! Telling an MCP server to stop when nobody is waiting for the answer.
//!
//! #200 Phase 1 gives tools a per-call time budget enforced at the dispatch
//! choke point with `tokio::time::timeout_at`, which *drops* the tool's future
//! when the budget runs out. For a tool that only holds local state that is
//! harmless. For an MCP tool it is not: the `tools/call` request has already
//! been written to the server, and rmcp installs no `Drop` hook on
//! [`RequestHandle`] — `await_response` sends `notifications/cancelled` only
//! when its own timeout fires. Drop the future without saying anything and the
//! server keeps grinding on a call whose result nobody will ever read, which on
//! a server with a small worker pool is how one abandoned request turns into a
//! wedged connection.
//!
//! So an MCP call is awaited through [`await_response_cancel_on_drop`], which
//! arms a guard for the whole wait and sends the cancellation whenever the wait
//! ends without a response — the budget expiring, the caller going away, or a
//! panic unwinding through it.

use std::time::Duration;

use rmcp::model::{CancelledNotification, CancelledNotificationParam, RequestId, ServerResult};
use rmcp::service::{Peer, RequestHandle, RoleClient, ServiceError};

/// Reason reported to the server. Servers surface this in their logs, so it
/// says who gave up and why rather than just "cancelled".
const DROPPED_REASON: &str = "archon: caller stopped waiting for this tool call";

/// Await an in-flight MCP request, cancelling it if the wait ends early.
///
/// `None` means `budget` expired. In that case — and if this future is dropped
/// before it resolves at all — the server is sent `notifications/cancelled` for
/// the request id before this returns.
pub(crate) async fn await_response_cancel_on_drop(
    handle: RequestHandle<RoleClient>,
    budget: Duration,
) -> Option<Result<ServerResult, ServiceError>> {
    let RequestHandle { rx, peer, id, .. } = handle;
    let mut guard = CancelOnDrop { peer, id: Some(id) };

    let received = tokio::time::timeout(budget, rx).await.ok()?;
    // The wait is over either way: a closed channel means the connection is
    // gone, so there is nothing left to cancel and nothing to send it over.
    guard.disarm();
    Some(received.unwrap_or(Err(ServiceError::TransportClosed)))
}

/// Sends `notifications/cancelled` unless disarmed first.
struct CancelOnDrop {
    peer: Peer<RoleClient>,
    id: Option<RequestId>,
}

impl CancelOnDrop {
    /// Called once the response is in hand: the server is already done, so
    /// cancelling would be a lie.
    fn disarm(&mut self) {
        self.id = None;
    }
}

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        let Some(request_id) = self.id.take() else {
            return;
        };

        // `send_notification` is async and `drop` is not, so the notification
        // has to outlive this frame on a task of its own. There is always a
        // runtime here in practice — the future being dropped was being polled
        // by one — but a caller dropping the future from a plain thread would
        // otherwise get a panic out of a destructor, so check rather than
        // assume, and say so loudly if the cancellation could not be sent.
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(
                request_id = ?request_id,
                "MCP call dropped outside a tokio runtime; the server was not told to cancel \
                 and will keep working on the request"
            );
            return;
        };

        let peer = self.peer.clone();
        runtime.spawn(async move {
            let notification = CancelledNotification::new(CancelledNotificationParam {
                request_id,
                reason: Some(DROPPED_REASON.to_string()),
            });
            if let Err(error) = peer.send_notification(notification.into()).await {
                tracing::debug!(%error, "could not deliver MCP cancellation notification");
            }
        });
    }
}
