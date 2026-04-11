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
//! ## Spec-vs-reality divergence #4 (approved phase-0 2026-04-11)
//!
//! Tests (b2) and (c) in the spec say "use `archon_test_support::
//! MockMemoryTrait`". That mock implements a LOCAL trait
//! `MemoryTraitLike` (2-arg async `store_memory(content, tags)`) which
//! was kept intentionally decoupled from the real `archon_memory::
//! MemoryTrait` (7-arg sync `store_memory`, 13 methods total) — per
//! TASK-AGS-008 spec §"verified visually against the real crate, not
//! imported, to keep this a test-only sibling".
//!
//! The real `save_agent_memory` takes `&dyn archon_memory::MemoryTrait`,
//! so `MockMemoryTrait` cannot satisfy the bound. Spec 008 and spec 010
//! are therefore internally inconsistent.
//!
//! **Resolution**: inline a `RecordingMemory` in THIS test file that
//! implements the real 12-method `MemoryTrait` (the trait's own doc
//! comment at `archon-memory/src/access.rs:20` says "13 public
//! operations" but the actual declaration has 12 fn items — a minor
//! doc drift in production code that phase-0 intentionally does NOT
//! touch). Eleven of the methods are `unimplemented!()` because
//! `save_agent_memory` only calls `store_memory`. This mirrors what
//! `MockMemoryTrait` *would* do if it were wired to the real trait,
//! but without forcing archon-test-support to take a dev-dep on
//! archon-memory (and the cozo build chain it pulls in transitively).

use std::sync::{Arc, Mutex};

use archon_core::agents::definition::AgentMemoryScope;
use archon_core::agents::memory::save_agent_memory;
use archon_memory::MemoryTrait;
use archon_memory::types::{Memory, MemoryError, MemoryType, RelType, SearchFilter};

// --------------------------------------------------------------------
// Inline RecordingMemory — satisfies the real 13-method MemoryTrait.
// Only store_memory is exercised; every other method panics so the
// test will fail loudly if a refactor starts calling a different API.
// --------------------------------------------------------------------

#[derive(Debug, Clone)]
struct StoredCall {
    content: String,
    title: String,
    memory_type: MemoryType,
    importance: f64,
    tags: Vec<String>,
    source_type: String,
    project_path: String,
}

#[derive(Default)]
struct RecordingMemory {
    calls: Arc<Mutex<Vec<StoredCall>>>,
}

impl RecordingMemory {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<StoredCall> {
        self.calls.lock().unwrap().clone()
    }
}

impl MemoryTrait for RecordingMemory {
    fn store_memory(
        &self,
        content: &str,
        title: &str,
        memory_type: MemoryType,
        importance: f64,
        tags: &[String],
        source_type: &str,
        project_path: &str,
    ) -> Result<String, MemoryError> {
        self.calls.lock().unwrap().push(StoredCall {
            content: content.to_string(),
            title: title.to_string(),
            memory_type,
            importance,
            tags: tags.to_vec(),
            source_type: source_type.to_string(),
            project_path: project_path.to_string(),
        });
        Ok("recorded-id".to_string())
    }

    fn get_memory(&self, _id: &str) -> Result<Memory, MemoryError> {
        unimplemented!("RecordingMemory: get_memory not used by save_agent_memory")
    }

    fn update_memory(
        &self,
        _id: &str,
        _content: Option<&str>,
        _tags: Option<&[String]>,
    ) -> Result<(), MemoryError> {
        unimplemented!("RecordingMemory: update_memory not used by save_agent_memory")
    }

    fn update_importance(&self, _id: &str, _importance: f64) -> Result<(), MemoryError> {
        unimplemented!("RecordingMemory: update_importance not used by save_agent_memory")
    }

    fn delete_memory(&self, _id: &str) -> Result<(), MemoryError> {
        unimplemented!("RecordingMemory: delete_memory not used by save_agent_memory")
    }

    fn create_relationship(
        &self,
        _from_id: &str,
        _to_id: &str,
        _rel_type: RelType,
        _context: Option<&str>,
        _strength: f64,
    ) -> Result<(), MemoryError> {
        unimplemented!("RecordingMemory: create_relationship not used by save_agent_memory")
    }

    fn recall_memories(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("RecordingMemory: recall_memories not used by save_agent_memory")
    }

    fn search_memories(&self, _filter: &SearchFilter) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("RecordingMemory: search_memories not used by save_agent_memory")
    }

    fn list_recent(&self, _limit: usize) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("RecordingMemory: list_recent not used by save_agent_memory")
    }

    fn memory_count(&self) -> Result<usize, MemoryError> {
        unimplemented!("RecordingMemory: memory_count not used by save_agent_memory")
    }

    fn clear_all(&self) -> Result<usize, MemoryError> {
        unimplemented!("RecordingMemory: clear_all not used by save_agent_memory")
    }

    fn get_related_memories(&self, _id: &str, _depth: u32) -> Result<Vec<Memory>, MemoryError> {
        unimplemented!("RecordingMemory: get_related_memories not used by save_agent_memory")
    }
}

// --------------------------------------------------------------------
// Test (a): three call sites in agent.rs
// --------------------------------------------------------------------

/// REGRESSION GUARD: DO NOT RELAX. See REQ-FOR-PRESERVE-D8 (a).
#[test]
fn test_save_agent_memory_invoked_at_all_three_call_sites() {
    let source = include_str!("../src/agent.rs");

    // Count non-comment occurrences of `save_agent_memory(`. We accept
    // both bare `save_agent_memory(` and the qualified
    // `crate::agents::memory::save_agent_memory(` form that the current
    // agent.rs uses — the `save_agent_memory(` suffix is the invariant.
    let count = source
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            // Skip comment lines.
            if trimmed.starts_with("//") || trimmed.starts_with("*") || trimmed.starts_with("///")
            {
                return false;
            }
            trimmed.contains("save_agent_memory(")
        })
        .count();

    assert!(
        count >= 3,
        "REQ-FOR-PRESERVE-D8 (a) violated: expected ≥3 save_agent_memory( call sites in \
         agent.rs, found {count}. If a call site was refactored away, the subagent-completion \
         memory-persistence path is broken."
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
    let memory = RecordingMemory::new();
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
    let memory = RecordingMemory::new();

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
/// This test intentionally ships `#[ignore]` on HEAD — the snapshot
/// file is owned by TASK-AGS-011 (`cargo public-api` baseline) and
/// lands in a later phase-0 task. Once that task merges, remove the
/// `#[ignore]` attribute and this guard will flip green.
#[test]
#[ignore = "waits on TASK-AGS-011 cargo public-api snapshot"]
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
