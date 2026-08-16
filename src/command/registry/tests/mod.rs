use super::*;
use std::sync::Arc;

/// Count of registered slash-command primaries.
/// 85 = 84 + `/draft` (FCDP in-session drafting command).
/// 87 = 86 + `/requirements` (Phase 6 traceability report surface).
/// 89 = 88 + `/worktrees` (#184 M7 isolated-agent review and merge).
const EXPECTED_COMMAND_COUNT: usize = 89;

mod aliases_core;
mod aliases_more;
mod basic;
mod emit;
mod integration;
