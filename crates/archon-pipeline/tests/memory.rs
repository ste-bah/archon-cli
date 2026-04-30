//! Tests for TASK-PIPE-A03: Native Memory Operations
//!
//! These tests verify that the `archon_pipeline::memory` module provides
//! direct MemoryGraph operations (no MCP tool calls) for:
//! - Storing agent outputs with structured metadata
//! - Recalling relevant memories for an agent
//! - Searching memories with pipeline-aware filters
//! - Creating inter-agent relationships
//!
//! All tests use `MemoryGraph::in_memory()` so no on-disk state is needed.

use chrono::Utc;

use archon_memory::MemoryGraph;
use archon_pipeline::memory::{
    AgentOutputMetadata, MemoryFilters, create_agent_relationship, recall_for_agent,
    search_for_prompt, store_agent_output,
};
use archon_pipeline::runner::PipelineType;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a default `AgentOutputMetadata` for the Coding pipeline.
fn coding_metadata(session_id: &str, phase: u32, tags: Vec<String>) -> AgentOutputMetadata {
    AgentOutputMetadata {
        pipeline_type: PipelineType::Coding,
        phase,
        session_id: session_id.to_string(),
        timestamp: Utc::now(),
        quality_score: 0.85,
        tags,
    }
}

/// Build a default `AgentOutputMetadata` for the Research pipeline.
fn research_metadata(session_id: &str, phase: u32, tags: Vec<String>) -> AgentOutputMetadata {
    AgentOutputMetadata {
        pipeline_type: PipelineType::Research,
        phase,
        session_id: session_id.to_string(),
        timestamp: Utc::now(),
        quality_score: 0.90,
        tags,
    }
}

/// Shorthand for an empty `MemoryFilters`.
fn empty_filters() -> MemoryFilters {
    MemoryFilters {
        pipeline_type: None,
        phase: None,
        agent_keys: vec![],
        tags: vec![],
        time_range: None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 1. store_agent_output returns a non-empty MemoryId.
#[tokio::test]
async fn test_store_agent_output_returns_memory_id() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph should init");
    let meta = coding_metadata("sess-001", 1, vec!["pipeline".into(), "coding".into()]);

    let mem_id = store_agent_output(
        &graph,
        "sess-001",
        "task-analyzer",
        "Analyzed the task requirements and produced a breakdown.",
        &meta,
    )
    .await
    .expect("store_agent_output should succeed");

    assert!(
        !mem_id.0.is_empty(),
        "MemoryId should be non-empty after storing agent output"
    );
}

/// 2. Store an output, then recall it — verifying round-trip content retrieval.
#[tokio::test]
async fn test_store_and_recall_round_trip() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph should init");
    let meta = coding_metadata("sess-002", 1, vec!["pipeline".into(), "coding".into()]);

    let content = "The user wants a REST API with three endpoints.";
    store_agent_output(&graph, "sess-002", "task-analyzer", content, &meta)
        .await
        .expect("store should succeed");

    let recalled = recall_for_agent(&graph, "task-analyzer", "REST API endpoints", 10)
        .await
        .expect("recall should succeed");

    assert!(
        !recalled.is_empty(),
        "recall_for_agent should return at least one entry after storing"
    );

    // At least one returned entry should contain the original content.
    let found = recalled
        .iter()
        .any(|entry| entry.content.contains("REST API"));
    assert!(
        found,
        "Recalled entries should include the previously stored content"
    );
}

/// 3. Stored memories carry the correct tags derived from pipeline type and agent key.
#[tokio::test]
async fn test_stored_memories_have_correct_tags() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph should init");
    let meta = coding_metadata(
        "sess-003",
        1,
        vec!["pipeline".into(), "coding".into(), "task-analyzer".into()],
    );

    store_agent_output(
        &graph,
        "sess-003",
        "task-analyzer",
        "Output with specific tags for verification.",
        &meta,
    )
    .await
    .expect("store should succeed");

    // Recall and check tags on the returned entries.
    let recalled = recall_for_agent(&graph, "task-analyzer", "tags verification", 10)
        .await
        .expect("recall should succeed");

    assert!(!recalled.is_empty(), "Should recall at least one entry");

    let entry = &recalled[0];
    assert!(
        entry.tags.contains(&"pipeline".to_string()),
        "Tags should include 'pipeline', got: {:?}",
        entry.tags
    );
    assert!(
        entry.tags.contains(&"coding".to_string()),
        "Tags should include 'coding', got: {:?}",
        entry.tags
    );
    assert!(
        entry.tags.contains(&"task-analyzer".to_string()),
        "Tags should include 'task-analyzer', got: {:?}",
        entry.tags
    );
}

/// 4. recall_for_agent respects the `limit` parameter.
#[tokio::test]
async fn test_recall_respects_limit() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph should init");

    // Store 5 distinct outputs.
    for i in 0..5 {
        let meta = coding_metadata("sess-004", 1, vec!["pipeline".into()]);
        store_agent_output(
            &graph,
            "sess-004",
            "requirement-extractor",
            &format!("Requirement item number {} for the feature.", i),
            &meta,
        )
        .await
        .expect("store should succeed");
    }

    let recalled = recall_for_agent(&graph, "requirement-extractor", "requirement feature", 2)
        .await
        .expect("recall should succeed");

    assert!(
        recalled.len() <= 2,
        "recall with limit=2 should return at most 2 entries, got {}",
        recalled.len()
    );
}

/// 5. search_for_prompt returns results that match the query.
#[tokio::test]
async fn test_search_for_prompt_returns_results() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph should init");
    let meta = coding_metadata("sess-005", 2, vec!["pipeline".into(), "coding".into()]);

    store_agent_output(
        &graph,
        "sess-005",
        "architect",
        "The system architecture uses a layered design with dependency injection.",
        &meta,
    )
    .await
    .expect("store should succeed");

    let results = search_for_prompt(&graph, "layered architecture", &empty_filters())
        .await
        .expect("search should succeed");

    assert!(
        !results.is_empty(),
        "search_for_prompt should find results matching 'layered architecture'"
    );

    let found = results
        .iter()
        .any(|entry| entry.content.contains("layered design"));
    assert!(
        found,
        "Search results should include the stored architecture description"
    );
}

/// 6. create_agent_relationship links two agent outputs without error.
#[tokio::test]
async fn test_create_agent_relationship() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph should init");

    // Store two agent outputs.
    let meta1 = coding_metadata("sess-006", 1, vec!["pipeline".into()]);
    let _id1 = store_agent_output(
        &graph,
        "sess-006",
        "task-analyzer",
        "Analysis: the task requires a new database schema.",
        &meta1,
    )
    .await
    .expect("store agent-1 output should succeed");

    let meta2 = coding_metadata("sess-006", 2, vec!["pipeline".into()]);
    let _id2 = store_agent_output(
        &graph,
        "sess-006",
        "architect",
        "Architecture: design the schema with three tables.",
        &meta2,
    )
    .await
    .expect("store agent-2 output should succeed");

    // Create a relationship between the two agents' outputs.
    let result = create_agent_relationship(
        &graph,
        "task-analyzer",
        "architect",
        "derived_from",
        "sess-006",
    )
    .await;

    assert!(
        result.is_ok(),
        "create_agent_relationship should succeed, got: {:?}",
        result.err()
    );
}

/// 7. search_for_prompt with tag filters returns only matching entries.
#[tokio::test]
async fn test_memory_filters_by_tags() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph should init");

    // Store a coding pipeline output.
    let coding_meta = coding_metadata(
        "sess-007",
        1,
        vec!["pipeline".into(), "coding".into(), "backend".into()],
    );
    store_agent_output(
        &graph,
        "sess-007",
        "backend-implementer",
        "Implemented the backend REST controller.",
        &coding_meta,
    )
    .await
    .expect("store coding output should succeed");

    // Store a research pipeline output.
    let research_meta = research_metadata(
        "sess-007",
        1,
        vec!["pipeline".into(), "research".into(), "survey".into()],
    );
    store_agent_output(
        &graph,
        "sess-007",
        "source-gatherer",
        "Gathered 12 research papers on the topic.",
        &research_meta,
    )
    .await
    .expect("store research output should succeed");

    // Search with a tag filter for "backend" — should return only the coding entry.
    let filters = MemoryFilters {
        pipeline_type: None,
        phase: None,
        agent_keys: vec![],
        tags: vec!["backend".into()],
        time_range: None,
    };

    let results = search_for_prompt(&graph, "pipeline", &filters)
        .await
        .expect("search with tag filter should succeed");

    // Every returned entry should have the "backend" tag.
    for entry in &results {
        assert!(
            entry.tags.contains(&"backend".to_string()),
            "Filtered results should only contain entries tagged 'backend', got tags: {:?}",
            entry.tags
        );
    }

    // The research entry should NOT appear.
    let has_research = results
        .iter()
        .any(|entry| entry.content.contains("research papers"));
    assert!(
        !has_research,
        "Tag-filtered search should not return entries without the 'backend' tag"
    );
}

/// 8. Verify the memory module uses MemoryGraph directly (no MCP tool calls).
///
/// This is a conceptual/structural test: we call every public function in the
/// memory module with a real in-memory MemoryGraph and confirm they all work.
/// If the module were using MCP tool calls instead of direct MemoryGraph methods,
/// these calls would fail because no MCP server is running.
#[tokio::test]
async fn test_no_mcp_tool_calls() {
    let graph = MemoryGraph::in_memory().expect("in-memory graph should init");
    let meta = coding_metadata("sess-008", 1, vec!["pipeline".into(), "direct".into()]);

    // store_agent_output — direct MemoryGraph, no MCP
    let id = store_agent_output(
        &graph,
        "sess-008",
        "verifier",
        "Direct memory storage without MCP.",
        &meta,
    )
    .await
    .expect("store_agent_output should work without MCP server");

    assert!(!id.0.is_empty(), "Should get a valid MemoryId");

    // recall_for_agent — direct MemoryGraph, no MCP
    let recalled = recall_for_agent(&graph, "verifier", "direct memory", 5)
        .await
        .expect("recall_for_agent should work without MCP server");

    assert!(
        !recalled.is_empty(),
        "recall should return entries without MCP"
    );

    // search_for_prompt — direct MemoryGraph, no MCP
    let results = search_for_prompt(&graph, "direct storage", &empty_filters())
        .await
        .expect("search_for_prompt should work without MCP server");

    // We don't assert non-empty here because keyword search may not match,
    // but the call itself must succeed without MCP.
    let _ = results;

    // create_agent_relationship — direct MemoryGraph, no MCP
    // Store a second output so we have two agents to link.
    let meta2 = coding_metadata("sess-008", 2, vec!["pipeline".into(), "direct".into()]);
    store_agent_output(
        &graph,
        "sess-008",
        "reviewer",
        "Review of the direct storage approach.",
        &meta2,
    )
    .await
    .expect("second store should work without MCP server");

    create_agent_relationship(&graph, "verifier", "reviewer", "related_to", "sess-008")
        .await
        .expect("create_agent_relationship should work without MCP server");
}
