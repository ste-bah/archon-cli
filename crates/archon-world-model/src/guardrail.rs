//! Runtime world-model guardrail contracts.
//!
//! These types are intentionally independent from the CLI/session loop. They
//! model the durable "predict -> guard -> verify -> learn" records used by
//! interactive sessions, tool runs, and pipeline steps.

include!("guardrail/00_types.rs");
include!("guardrail/01_decision.rs");
include!("guardrail/02_io_helpers.rs");
include!("guardrail/03_tests.rs");
