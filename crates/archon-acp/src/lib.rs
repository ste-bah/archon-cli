//! Agent Client Protocol over stdio (#189 Phase 11).
//!
//! ACP lets an editor drive an agent without a per-editor extension: the editor
//! speaks one protocol, and any conforming agent can be on the other end. This
//! crate is that end for archon.
//!
//! # Shape
//!
//! - [`protocol`] — the wire types, spelled the protocol's way rather than
//!   ours, with tests that assert on JSON so a rename cannot silently break
//!   every client.
//! - [`jsonrpc`] — newline-delimited JSON-RPC 2.0 framing.
//! - [`peer`] — the client as this side can address it, including the
//!   request/reply correlation that makes `session/request_permission` a
//!   question rather than a broadcast.
//! - [`serve`] — the loop.
//! - [`agent`] — the trait the binary implements.
//!
//! # Why there are no `archon-*` dependencies
//!
//! The protocol is the part with an external specification to conform to, and
//! conformance is much easier to keep when it can be tested in milliseconds
//! against a stub agent. Keeping this crate a leaf also stops the wire format
//! growing an opinion about one particular agent's internals, which is what
//! would make the next protocol revision painful.

pub mod agent;
pub mod jsonrpc;
pub mod peer;
pub mod protocol;
pub mod serve;

pub use agent::AcpAgent;
pub use peer::Peer;
pub use protocol::{
    ContentBlock, SessionUpdate, StopReason, ToolCallContent, ToolKind, ToolStatus,
};
pub use serve::serve;
