use super::*;
use std::sync::Arc;

/// Count of registered slash-command primaries.
/// 85 = 84 + `/draft` (FCDP in-session drafting command).
/// 87 = 86 + `/requirements` (Phase 6 traceability report surface).
const EXPECTED_COMMAND_COUNT: usize = 87;

mod aliases_core;
mod aliases_more;
mod basic;
mod emit;
mod integration;
