//! Library target for the archon-cli-workspace root crate.
//!
//! Exposes `cli_args` so integration tests can verify clap parsing
//! without depending on the binary entry point.

pub mod cli_args;
pub mod event_coalescer;
