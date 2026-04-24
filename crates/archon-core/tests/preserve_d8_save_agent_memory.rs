//! REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D8.
//!
//! Locks in the pre-refactor behaviour of AGT-026/AGT-027 "save_agent_memory
//! on subagent completion" so that any phase-1 refactor that drifts from
//! the current semantics is caught immediately.
//!
//! Invariants asserted (from 00-prd-analysis.md REQ-FOR-PRESERVE-D8,
//! 02-technical-spec.md §1538, §1594-1597):
//!   (a) save_agent_memory is invoked at THREE call sites in agent.rs
//!   (b1) neither agents/memory.rs nor archon-memory/lib.rs reference
//!        any `mcp__memorygraph__` symbol (belt-and-braces check over
//!        TASK-AGS-003's banned-imports guard)
//!   (b2) with memory_scope = Some(scope), save_agent_memory calls
//!        `archon_memory::MemoryTrait::store_memory` exactly once
//!   (c)  with memory_scope = None, store_memory is NOT called (AC-101
//!        no-op path)
//!   (d)  the agents-memory public-API snapshot file exists — GATED
//!        behind #[ignore] until TASK-AGS-011 lands the snapshot
//!   (e)  archon-memory crate still uses `cozo::DbInstance` as its
//!        storage backend
//!
//! ## Spec-vs-reality divergence #4 (RESOLVED 2026-04-11)
//!
//! Originally this test inlined a 100-line `RecordingMemory` double
//! because `archon_test_support::MockMemoryTrait` had been defined
//! against a local 1-method trait instead of the real 12-method
//! `archon_memory::MemoryTrait`. Forensic audit of TASK-AGS-008
//! identified the mock as unusable for its stated purpose; the fix
//! landed the real trait impl in archon-test-support and this file
//! now uses `MockMemoryTrait` directly as the spec originally
//! intended.

use archon_core::agents::definition::AgentMemoryScope;
use archon_core::agents::memory::save_agent_memory;
use archon_memory::types::MemoryType;
use archon_test_support::memory::MockMemoryTrait;

// --------------------------------------------------------------------
// Test (a): three call sites in agent.rs
// --------------------------------------------------------------------

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D8 (a).
///
/// TASK-AGS-105: the three legacy `save_agent_memory` call sites in agent.rs
/// (foreground subagent result, background-result arrival, and
/// handle_subagent_result) were intentionally collapsed into a single call
/// in `AgentSubagentExecutor::on_inner_complete` inside
/// `archon-core/src/subagent_executor.rs`. The invariant is now "at least
/// one call site in subagent_executor.rs, zero in agent.rs" — the old
/// indirection MUST NOT be reintroduced.
#[test]
fn test_save_agent_memory_invoked_at_all_three_call_sites() {
    let agent_source = include_str!("../src/agent.rs");
    let exec_source = include_str!("../src/subagent_executor.rs");

    // Count non-comment occurrences of `save_agent_memory(` in
    // subagent_executor.rs. Must be >= 1 (collapsed call site).
    let exec_count = exec_source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("*") || trimmed.starts_with("///") {
                return false;
            }
            trimmed.contains("save_agent_memory(")
        })
        .count();

    assert!(
        exec_count >= 1,
        "REQ-FOR-PRESERVE-D8 (a) violated: expected ≥1 save_agent_memory( call site in \
         subagent_executor.rs, found {exec_count}. The collapsed call site in \
         on_inner_complete is the post-TASK-AGS-105 persistence path."
    );

    // agent.rs must not reintroduce the old indirection. Zero non-comment
    // call sites there.
    let agent_count = agent_source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") || trimmed.starts_with("*") || trimmed.starts_with("///") {
                return false;
            }
            trimmed.contains("save_agent_memory(")
        })
        .count();

    assert_eq!(
        agent_count, 0,
        "REQ-FOR-PRESERVE-D8 (a) violated: agent.rs reintroduced {agent_count} \
         save_agent_memory( call site(s). TASK-AGS-105 collapsed all persistence calls \
         into subagent_executor.rs::on_inner_complete; agent.rs must stay empty of them."
    );
}

// --------------------------------------------------------------------
// Test (b1): no mcp__memorygraph__ references
// --------------------------------------------------------------------

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D8 (b1).
#[test]
fn test_no_mcp_memorygraph_references() {
    let memory_rs = include_str!("../src/agents/memory.rs");
    let archon_memory_lib = include_str!("../../archon-memory/src/lib.rs");

    assert!(
        !memory_rs.contains("mcp__memorygraph__"),
        "REQ-FOR-PRESERVE-D8 (b1) violated: agents/memory.rs references mcp__memorygraph__ — \
         the memory store must be archon_memory::MemoryTrait, never an MCP shim."
    );
    assert!(
        !archon_memory_lib.contains("mcp__memorygraph__"),
        "REQ-FOR-PRESERVE-D8 (b1) violated: archon-memory/lib.rs references mcp__memorygraph__."
    );
}

// --------------------------------------------------------------------
// Test (b2): Some(scope) → store_memory called once
// --------------------------------------------------------------------

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D8 (b2).
#[test]
fn test_store_memory_called_when_scope_is_some() {
    let memory = MockMemoryTrait::new();
    let scope = AgentMemoryScope::Project;
    let extra_tags = vec!["from-test".to_string()];

    let result = save_agent_memory(
        "researcher",
        "content-body",
        "test-title",
        &extra_tags,
        &memory,
        "/tmp/test-project",
        Some(&scope),
    );
    assert!(result.is_ok(), "save_agent_memory returned Err: {result:?}");

    let calls = memory.calls();
    assert_eq!(
        calls.len(),
        1,
        "REQ-FOR-PRESERVE-D8 (b2) violated: expected exactly 1 store_memory call, got {}",
        calls.len()
    );

    let call = &calls[0];
    assert_eq!(call.content, "content-body");
    assert_eq!(call.title, "test-title");
    assert_eq!(call.source_type, "agent");
    assert_eq!(call.project_path, "/tmp/test-project");
    // AGT-027 invariants on the store_memory arguments.
    assert!(matches!(call.memory_type, MemoryType::Fact));
    assert!(
        (call.importance - 0.5).abs() < f64::EPSILON,
        "importance drifted from 0.5: {}",
        call.importance
    );
    // Tags must include both scoping tags plus the extra tag, and
    // NOTHING ELSE — a refactor that silently injects debug/leak tags
    // would be caught by the length check (adversarial-review fix,
    // 2026-04-11).
    assert_eq!(
        call.tags.len(),
        3,
        "expected exactly 3 tags (agent, scope, extra), got {:?}",
        call.tags
    );
    assert!(call.tags.contains(&"agent:researcher".to_string()));
    assert!(call.tags.contains(&"scope:project".to_string()));
    assert!(call.tags.contains(&"from-test".to_string()));
}

// --------------------------------------------------------------------
// Test (c): None → no-op
// --------------------------------------------------------------------

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D8 (c).
#[test]
fn test_memory_scope_none_is_no_op() {
    let memory = MockMemoryTrait::new();

    let result = save_agent_memory(
        "researcher",
        "content-body",
        "test-title",
        &[],
        &memory,
        "/tmp/test-project",
        None,
    );
    assert!(
        result.is_ok(),
        "None-scope path must return Ok, got {result:?}"
    );

    let calls = memory.calls();
    assert!(
        calls.is_empty(),
        "REQ-FOR-PRESERVE-D8 (c) violated: memory_scope = None must NOT call store_memory; \
         got {} call(s). AC-101 no-persistent-memory path is broken.",
        calls.len()
    );
}

// --------------------------------------------------------------------
// Test (d): public-API snapshot — gated on TASK-AGS-011
// --------------------------------------------------------------------

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D8 (d).
///
/// TASK-AGS-011 has landed the snapshot; this guard is now live. It
/// simply asserts the fixture file exists on disk — the live drift
/// check is in `crates/archon-core/tests/public_api_snapshot.rs`.
#[test]
fn test_agents_memory_api_snapshot_exists() {
    let candidates = [
        "tests/fixtures/baseline/agents_memory_api.txt",
        "../../tests/fixtures/baseline/agents_memory_api.txt",
        "crates/archon-core/tests/fixtures/baseline/agents_memory_api.txt",
    ];
    let found = candidates.iter().any(|p| std::path::Path::new(p).exists());
    assert!(
        found,
        "REQ-FOR-PRESERVE-D8 (d) violated: agents_memory_api.txt snapshot not found in any \
         of {candidates:?}. TASK-AGS-011 is responsible for this file."
    );
}

// --------------------------------------------------------------------
// Test (e): archon-memory still uses cozo::DbInstance
// --------------------------------------------------------------------

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D8 (e).
#[test]
fn test_archon_memory_uses_cozo_dbinstance() {
    // `graph.rs` is the concrete CozoDB owner; `lib.rs` only re-exports
    // the trait and types and does NOT itself mention cozo. We grep the
    // concrete owner to prove the crate still carries the cozo backend.
    let graph_src = include_str!("../../archon-memory/src/graph.rs");

    let has_dbinstance = graph_src.contains("cozo::DbInstance") || graph_src.contains("DbInstance");
    let has_use_cozo = graph_src.contains("use cozo");
    assert!(
        has_dbinstance && has_use_cozo,
        "REQ-FOR-PRESERVE-D8 (e) violated: archon-memory/graph.rs no longer imports cozo or \
         uses DbInstance. has_dbinstance={has_dbinstance}, has_use_cozo={has_use_cozo}. The \
         storage backend must remain CozoDB until an explicit migration task lands."
    );
}
