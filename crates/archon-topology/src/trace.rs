//! The ambient trace: an append-only jsonl event log per graph.
//!
//! # Why files and not the database
//!
//! The Cozo stores are SQLite-backed behind a process-wide write lock keyed by
//! canonicalized database path (`archon-cozo`). A write from a fan-out worker
//! therefore serializes every other writer in the process, and the guarded
//! retry budget parks a thread for up to ~19 seconds under contention. Tracing
//! is by definition on the hot path of every tool call, so it cannot touch the
//! database at all. It appends to a file instead, and a single batched fold
//! (milestone 2, `src/command/topology_fold.rs`) is the only writer.
//!
//! This crate has no `cozo` dependency, which makes that a compile-time
//! property rather than a convention someone has to remember.
//!
//! # Layout
//!
//! ```text
//! .archon/topology/
//!   <graph-id>/
//!     graph.json      the lowered TaskGraph, or a reconstructed skeleton
//!     trace.jsonl     append-only event log
//!     ingested        marker, written last by a successful fold
//! ```
//!
//! Resolving that layout is [`paths`]; the record type is [`record`]; the two
//! ends of the file are [`writer`] and [`reader`].
//!
//! # Concurrency contract
//!
//! [`TraceWriter::append`] serializes the record and its terminating newline
//! into one buffer and issues a single `write` against a handle opened in
//! append mode. Concurrent appenders therefore interleave whole lines and never
//! split one. Note that `archon-workflow`'s `WorkflowStore::append_event_line`
//! — cited by the design as the precedent — does *not* do this; it makes two
//! `write_all` calls, body then newline, and can interleave between them. That
//! is a latent defect there, not a pattern to copy.
//!
//! A reader running concurrently with appends must therefore treat a trailing
//! fragment as absent rather than as corruption; [`read_trace`] does.

mod paths;
mod reader;
mod record;
mod writer;

pub use paths::{
    GRAPH_FILE, INGESTED_MARKER, TOPOLOGY_DIR, TRACE_FILE, TopologyPaths, sanitize_graph_id,
};
pub use reader::{TraceReadout, read_trace};
pub use record::{TraceKind, TraceRecord};
pub use writer::{MAX_RECORD_BYTES, TraceWriter};

#[cfg(test)]
mod tests;
