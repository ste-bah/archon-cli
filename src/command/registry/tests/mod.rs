use super::*;
use std::sync::Arc;

/// Count of registered slash-command primaries.
/// 85 = 84 + `/draft` (FCDP in-session drafting command).
/// 87 = 86 + `/requirements` (Phase 6 traceability report surface).
/// 89 = 88 + `/worktrees` (#184 M7 isolated-agent review and merge).
/// 90 = 89 + `/feedback` (#193 Phase C per-message human feedback).
/// 91 = 90 + `/fork-at` (#192 fork from an earlier message).
/// 92 = 91 + `/session-ref` (#200 Phase 4 cross-session references).
const EXPECTED_COMMAND_COUNT: usize = 92;

mod aliases_core;
mod aliases_more;
mod basic;
mod emit;
mod integration;
