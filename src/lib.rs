#![allow(clippy::doc_lazy_continuation)]
#![allow(clippy::doc_overindented_list_items)]
#![allow(clippy::empty_line_after_doc_comments)]

//! Library target for the archon-cli-workspace root crate.
//!
//! Exposes `cli_args` so integration tests can verify clap parsing
//! without depending on the binary entry point.

pub mod cli_args;
pub mod event_coalescer;
