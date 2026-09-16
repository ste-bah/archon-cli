//! Memory tools for the workflow CLI's stage subagents.
//!
//! Issue-28 (run wf-719ff3b0, stage agents-12, TASK-AHDM-001): the task declared
//! `required_tools: ["memory_recall"]`, the host's acceptance check demanded
//! that name in `commands_run`, and the registry the stage subagents draw from
//! had never held it — `memory_recall`/`memory_store` were registered only for
//! interactive sessions (`src/session/interactive_setup.rs`), while
//! `pipeline_support::install_workflow_cli_subagent_executor` built its registry
//! from `create_default_registry` plus the project MCP tools. Admitting the name
//! into the stage's allowed list (`workflow_live_mcp::declared_native_tools`)
//! is half the fix; this is the other half, because `clone_filtered` can only
//! offer what the registry holds.
//!
//! Split out of `pipeline_support.rs` for the 500-line ceiling, not because the
//! ownership changed: the executor install is still the one caller.

use std::sync::Arc;

use archon_core::config::ArchonConfig;
use archon_core::dispatch::ToolRegistry;
use archon_memory::MemoryTrait;

/// Open the configured memory and register the memory tools, when `[memory]
/// enabled` says the model may have them.
///
/// The open mirrors `interactive_bootstrap`: the same `open_spec()`, the same
/// `open_configured_memory`, the same `Arc<MemoryAccess>` as `dyn MemoryTrait`.
/// A workflow run has usually opened this database once already for the task
/// board (`workflow_live_board`), so this open goes through the election and
/// comes back as a client of the server this process is already running —
/// which is what the election is for, and why the two opens do not collide.
///
/// Failure is loud and non-fatal. The TUI cannot degrade (its rules engine and
/// injector are built from the handle) and so refuses to start; a workflow run
/// can, and a stage without memory tools that says so in the log is a better
/// outcome than a run that never launched. The stage's own "never exercised"
/// check still catches a task that needed the tool and did not get it.
pub(crate) async fn register_memory_tools(config: &ArchonConfig, registry: &mut ToolRegistry) {
    if !config.memory.enabled {
        return;
    }
    let spec = config.memory.open_spec();
    let opened = match archon_memory::open_configured_memory(&spec).await {
        Ok(opened) => opened,
        Err(error) => {
            let (_, db_path) = spec.resolve_paths();
            tracing::warn!(
                %error,
                db_path = %db_path.display(),
                "memory unavailable: workflow stage subagents run without memory_recall/memory_store"
            );
            return;
        }
    };
    let db_path = opened.db_path.clone();
    let memory: Arc<dyn MemoryTrait> = Arc::new(opened.access);
    register_memory_tools_on(memory, registry);
    tracing::info!(db_path = %db_path.display(), "workflow stage memory tools registered");
}

/// The registration half, over an already-open handle.
///
/// Separate from the open so the pair of names can be checked against an
/// in-memory graph; `open_configured_memory` always resolves to a real path.
pub(crate) fn register_memory_tools_on(memory: Arc<dyn MemoryTrait>, registry: &mut ToolRegistry) {
    registry.register(Box::new(archon_tools::memory::MemoryStoreTool::new(
        Arc::clone(&memory),
    )));
    registry.register(Box::new(archon_tools::memory::MemoryRecallTool::new(
        memory,
    )));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_half_installs_both_memory_tool_names() {
        let memory: Arc<dyn MemoryTrait> =
            Arc::new(archon_memory::MemoryGraph::in_memory().expect("in-memory graph"));
        let mut registry = ToolRegistry::new();

        register_memory_tools_on(memory, &mut registry);

        assert!(registry.get("memory_recall").is_some(), "memory_recall");
        assert!(registry.get("memory_store").is_some(), "memory_store");
    }

    #[tokio::test]
    async fn disabled_memory_registers_nothing_and_opens_nothing() {
        // `[memory] enabled = false` withholds memory from the model; the
        // registry must come back untouched without the open being attempted.
        let mut config = ArchonConfig::default();
        config.memory.enabled = false;
        let mut registry = ToolRegistry::new();

        register_memory_tools(&config, &mut registry).await;

        assert!(registry.get("memory_recall").is_none());
        assert!(registry.get("memory_store").is_none());
    }
}
