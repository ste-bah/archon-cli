use super::*;
use std::sync::Arc;

use archon_consciousness::rules::RuleSource;
use archon_memory::MemoryGraph;

use crate::command::dispatcher::Dispatcher;
use crate::command::registry::{CommandContext, RegistryBuilder};

/// Build a `CommandContext` with a freshly-created channel and the
/// supplied `memory` handle. Mirrors the AGS-817 /memory + B18
/// /recall `make_ctx(memory)` fixture — DIRECT pattern, no
/// snapshot, no effect slot.
fn make_rules_ctx(
    memory: Option<Arc<dyn MemoryTrait>>,
) -> (CommandContext, archon_tui::event_channel::TuiEventReceiver) {
    // TASK-AGS-POST-6-SHARED-FIXTURES-V2: migrated to CtxBuilder.
    crate::command::test_support::CtxBuilder::new()
        .with_memory_opt(memory)
        .build()
}

/// Build a real in-memory MemoryGraph wrapped in an Arc<dyn>. Using
/// the real backend (rather than a StubMemory) exercises the full
/// RulesEngine round-trip (search_memories → update_memory /
/// delete_memory) in the same way the rules.rs unit tests do — see
/// `crates/archon-consciousness/src/rules.rs:334-337`
/// (`make_engine` helper) for the upstream pattern.
fn make_graph() -> Arc<MemoryGraph> {
    Arc::new(MemoryGraph::in_memory().expect("in-memory graph should succeed"))
}

/// R4: description is byte-identical to the `declare_handler!`
/// stub at registry.rs:1336. Any drift here means the stub and
/// the new handler have diverged.
mod cases_a;
mod cases_b;
