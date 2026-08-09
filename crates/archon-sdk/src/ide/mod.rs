//! IDE extension protocol layer for TASK-CLI-411.
//!
//! Implements a JSON-RPC 2.0 protocol for communication between IDE extensions
//! and the Archon agent. Supports both WebSocket and stdio transports.

pub mod config;
pub mod context_files;
pub mod events;
pub mod handler;
pub mod permission;
pub mod protocol;
pub mod runtime;
pub mod stdio;
