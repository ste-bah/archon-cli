//! Integration tests for the singleton memory server pattern.
//!
//! Covers protocol serialization, TCP server dispatch, client
//! round-trips, MemoryTrait implementations, and the access factory.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use archon_memory::MemoryGraph;
use archon_memory::access::{MemoryAccess, MemoryTrait, open_memory};
use archon_memory::client::MemoryClient;
use archon_memory::protocol::{Request, Response, make_request, parse_response};
use archon_memory::server::MemoryServer;
use archon_memory::types::{MemoryType, RelType, SearchFilter};

#[path = "memory_server_tests/board.rs"]
mod board;
#[path = "memory_server_tests/client.rs"]
mod client;
#[path = "memory_server_tests/factory.rs"]
mod factory;
#[path = "memory_server_tests/memory_trait.rs"]
mod memory_trait;
#[path = "memory_server_tests/protocol.rs"]
mod protocol;
#[path = "memory_server_tests/server.rs"]
mod server;
#[path = "memory_server_tests/support.rs"]
mod support;
