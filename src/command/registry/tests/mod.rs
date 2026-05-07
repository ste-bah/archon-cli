use super::*;
use std::sync::Arc;

/// Count of registered slash-command primaries.
const EXPECTED_COMMAND_COUNT: usize = 78;

mod aliases_core;
mod aliases_more;
mod basic;
mod emit;
mod integration;
