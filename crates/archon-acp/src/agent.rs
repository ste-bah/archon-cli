//! The seam between the protocol and whatever runs the turn (#189 Phase 11).
//!
//! This trait is why the crate has no `archon-*` dependencies. The wire format
//! can be tested against a stub in milliseconds, and the protocol cannot grow a
//! dependency on one particular agent's internals — which is the failure that
//! would make ACP support impossible to keep conformant later.

use std::sync::Arc;

use crate::peer::Peer;
use crate::protocol::StopReason;

/// What a client's requests are answered by.
#[async_trait::async_trait]
pub trait AcpAgent: Send + Sync {
    /// Open a session rooted at `cwd` and return its id.
    async fn new_session(&self, cwd: &str) -> anyhow::Result<String>;

    /// Run one turn.
    ///
    /// `peer` is how the turn reports itself: message chunks, tool calls, and
    /// permission requests all go back through it while this is running, which
    /// is why it is passed in rather than being reachable from the agent — a
    /// turn must not be able to talk to a client it was not invoked by.
    ///
    /// Returns why the turn ended. Returning [`StopReason::Cancelled`] is how
    /// a turn reports that it noticed [`AcpAgent::cancel`].
    async fn prompt(&self, session_id: &str, text: &str, peer: Arc<Peer>) -> StopReason;

    /// Stop the turn running in `session_id`.
    ///
    /// Synchronous and non-blocking on purpose: it is called from the reader
    /// loop, which must go straight back to reading. Signalling is all it does;
    /// the turn reports the outcome itself.
    fn cancel(&self, session_id: &str);
}
