use super::*;
use std::sync::Arc;

/// Count of registered slash-command primaries.
/// 85 = 84 + `/draft` (FCDP in-session drafting command).
const EXPECTED_COMMAND_COUNT: usize = 86;

mod aliases_core;
mod aliases_more;
mod basic;
mod emit;
mod integration;
