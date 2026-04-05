//! IDE extension protocol layer for TASK-CLI-411.
//!
//! Implements a JSON-RPC 2.0 protocol for communication between IDE extensions
//! and the Archon agent. Supports both WebSocket and stdio transports.

pub mod handler;
pub mod protocol;
pub mod stdio;
