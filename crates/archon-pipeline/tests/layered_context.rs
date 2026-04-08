//! Integration tests for TASK-PIPE-B07: Layered Context Loading (L0-L3).
//!
//! Tests the `LayeredContextLoader` which assembles four-tier memory context:
//! - L0 Identity (~100 tokens, always)
//! - L1 Essential Patterns (~500 tokens, always)
//! - L2 On-Demand (~200-500 tokens, per-agent, compressed)
//! - L3 Deep Search (unlimited, fallback only on quality retry)

use archon_pipeline::coding::rlm::RlmStore;
use archon_pipeline::layered_context::{
    AgentMemoryRequest, IdentityContext, LayeredContextLoader, PatternContext,
};
use archon_pipeline::prompt_cap::count_tokens;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Build a minimal IdentityContext for testing.
fn sample_identity() -> IdentityContext {
    IdentityContext {
        session_id: "sess-abc-123".to_string(),
        pipeline_type: "coding".to_string(),
        task_summary: "Implement user auth middleware".to_string(),
        agent_position: "Phase 3 / Agent 12 of 48".to_string(),
        wiring_obligations: Some("Output to coding/implementation/auth-middleware".to_string()),
    }
}

/// Build an IdentityContext without wiring obligations.
fn identity_no_wiring() -> IdentityContext {
    IdentityContext {
        session_id: "sess-xyz-789".to_string(),
        pipeline_type: "research".to_string(),
        task_summary: "Investigate caching strategies".to_string(),
        agent_position: "Phase 1 / Agent 3 of 24".to_string(),
        wiring_obligations: None,
    }
}

/// Build a PatternContext with SONA patterns, corrections, and decisions.
fn sample_patterns() -> PatternContext {
    PatternContext {
        sona_patterns: vec![
            "repo-pattern: data access layer".to_string(),
            "facade-pattern: orchestration".to_string(),
            "guard-clause: early return".to_string(),
            "builder-pattern: config construction".to_string(),
            "strategy-pattern: algorithm selection".to_string(),
        ],
        recent_corrections: vec![
            "Avoid unwrap in error paths".to_string(),
            "Use &str not String for read-only params".to_string(),
            "Prefer impl Trait over dyn Trait for single callers".to_string(),
        ],
        architectural_decisions: vec![
            "postgres for persistence".to_string(),
            "jwt for authentication".to_string(),
        ],
    }
}

/// Build an empty PatternContext (no SONA patterns).
fn empty_patterns() -> PatternContext {
    PatternContext {
        sona_patterns: vec![],
        recent_corrections: vec!["Fix lifetime annotations".to_string()],
        architectural_decisions: vec!["sqlite for dev".to_string()],
    }
}

/// Build a sample AgentMemoryRequest.
fn sample_agent_request() -> AgentMemoryRequest {
    AgentMemoryRequest {
        agent_key: "core-implementer".to_string(),
        memory_domains: vec![
            "coding/understanding/task-analysis".to_string(),
            "coding/design/api-design".to_string(),
        ],
        phase: 3,
    }
}

/// Build an AgentMemoryRequest with domains not in the RLM store.
fn missing_domains_request() -> AgentMemoryRequest {
    AgentMemoryRequest {
        agent_key: "frontend-implementer".to_string(),
        memory_domains: vec![
            "coding/frontend/components".to_string(),
            "coding/frontend/styles".to_string(),
        ],
        phase: 4,
    }
}

/// Build and populate an RlmStore with sample data.
fn sample_rlm_store() -> RlmStore {
    let mut store = RlmStore::new();
    store.write(
        "coding/understanding/task-analysis",
        "The task requires implementing an authentication middleware that validates JWT tokens, \
         checks user permissions against the database, and injects the authenticated user into \
         the request context. The middleware should handle expired tokens gracefully and return \
         appropriate HTTP status codes. It must integrate with the existing Express-style router.",
    );
    store.write(
        "coding/design/api-design",
        "API endpoints: POST /auth/login returns JWT, GET /auth/verify checks token validity, \
         DELETE /auth/logout invalidates token. Middleware function signature: \
         fn auth_middleware(req: Request, next: Next) -> Response. Token storage uses Redis \
         with 24h TTL. Rate limiting: 5 attempts per minute per IP.",
    );
    store.write(
        "coding/implementation/parser",
        "Parser implementation for token validation using jsonwebtoken crate with RS256. \
         Handles both access and refresh tokens with separate validation logic.",
    );
    // Domains that should NOT be loaded for sample_agent_request
    store.write(
        "coding/testing/unit-tests",
        "Test suite covering auth middleware: 15 tests for happy path, 8 for error handling.",
    );
    store
}

// ===========================================================================
// 1. test_default_budgets
// ===========================================================================

#[test]
fn test_default_budgets() {
    let loader = LayeredContextLoader::new();
    // Default budgets: L0=100, L1=500, L2=500
    // We verify by loading L0 and checking it respects the 100-token budget.
    let identity = sample_identity();
    let l0 = loader.load_l0(&identity);
    assert!(
        count_tokens(&l0) <= 100,
        "Default L0 budget should be 100 tokens, got {}",
        count_tokens(&l0)
    );
}

// ===========================================================================
// 2. test_custom_budgets
// ===========================================================================

#[test]
fn test_custom_budgets() {
    let loader = LayeredContextLoader::with_budgets(200, 300, 400);
    let identity = sample_identity();
    let patterns = sample_patterns();

    let l0 = loader.load_l0(&identity);
    let l1 = loader.load_l1(&patterns);

    assert!(
        count_tokens(&l0) <= 200,
        "Custom L0 budget of 200 not respected, got {} tokens",
        count_tokens(&l0)
    );
    assert!(
        count_tokens(&l1) <= 300,
        "Custom L1 budget of 300 not respected, got {} tokens",
        count_tokens(&l1)
    );
}

// ===========================================================================
// 3. test_l0_contains_identity_fields
// ===========================================================================

#[test]
fn test_l0_contains_identity_fields() {
    let loader = LayeredContextLoader::new();
    let identity = sample_identity();
    let l0 = loader.load_l0(&identity);

    assert!(
        l0.contains("sess-abc-123"),
        "L0 should contain session_id, got: {l0}"
    );
    assert!(
        l0.contains("Implement user auth middleware"),
        "L0 should contain task_summary, got: {l0}"
    );
    assert!(
        l0.contains("Phase 3 / Agent 12 of 48"),
        "L0 should contain agent_position, got: {l0}"
    );
}

// ===========================================================================
// 4. test_l0_under_budget
// ===========================================================================

#[test]
fn test_l0_under_budget() {
    let loader = LayeredContextLoader::new();
    let identity = sample_identity();
    let l0 = loader.load_l0(&identity);
    let tokens = count_tokens(&l0);

    assert!(
        tokens <= 100,
        "L0 should be under 100 tokens (default budget), got {tokens}"
    );
    // L0 should also be non-empty — it always loads.
    assert!(tokens > 0, "L0 should not be empty");
}

// ===========================================================================
// 5. test_l1_with_sona_patterns
// ===========================================================================

#[test]
fn test_l1_with_sona_patterns() {
    let loader = LayeredContextLoader::new();
    let patterns = sample_patterns();
    let l1 = loader.load_l1(&patterns);

    // All 5 SONA patterns should appear.
    assert!(
        l1.contains("repo-pattern"),
        "L1 should contain first SONA pattern"
    );
    assert!(
        l1.contains("facade-pattern"),
        "L1 should contain second SONA pattern"
    );
    assert!(
        l1.contains("guard-clause"),
        "L1 should contain third SONA pattern"
    );
    assert!(
        l1.contains("builder-pattern"),
        "L1 should contain fourth SONA pattern"
    );
    assert!(
        l1.contains("strategy-pattern"),
        "L1 should contain fifth SONA pattern"
    );
}

// ===========================================================================
// 6. test_l1_empty_sona_graceful
// ===========================================================================

#[test]
fn test_l1_empty_sona_graceful() {
    let loader = LayeredContextLoader::new();
    let patterns = empty_patterns();
    let l1 = loader.load_l1(&patterns);

    // Should still produce output with corrections and decisions.
    assert!(
        !l1.is_empty(),
        "L1 should not be empty even without SONA patterns"
    );
    assert!(
        l1.contains("Fix lifetime annotations"),
        "L1 should contain corrections even without SONA patterns"
    );
    assert!(
        l1.contains("sqlite for dev"),
        "L1 should contain architectural decisions even without SONA patterns"
    );
}

// ===========================================================================
// 7. test_l1_under_budget
// ===========================================================================

#[test]
fn test_l1_under_budget() {
    let loader = LayeredContextLoader::new();
    let patterns = sample_patterns();
    let l1 = loader.load_l1(&patterns);
    let tokens = count_tokens(&l1);

    assert!(
        tokens <= 500,
        "L1 should be under 500 tokens (default budget), got {tokens}"
    );
    assert!(tokens > 0, "L1 should not be empty");
}

// ===========================================================================
// 8. test_l0_plus_l1_under_600_tokens
// ===========================================================================

#[test]
fn test_l0_plus_l1_under_600_tokens() {
    let loader = LayeredContextLoader::new();
    let identity = sample_identity();
    let patterns = sample_patterns();

    let l0 = loader.load_l0(&identity);
    let l1 = loader.load_l1(&patterns);
    let combined = count_tokens(&l0) + count_tokens(&l1);

    assert!(
        combined <= 600,
        "L0 + L1 must be under 600 tokens (100 + 500 budgets), got {combined}"
    );
}

// ===========================================================================
// 9. test_l2_filters_by_memory_domains
// ===========================================================================

#[test]
fn test_l2_filters_by_memory_domains() {
    let loader = LayeredContextLoader::new();
    let request = sample_agent_request();
    let store = sample_rlm_store();
    let l2 = loader.load_l2(&request, &store);

    // L2 should include content from the requested domains.
    // The request asks for task-analysis and api-design.
    assert!(
        l2.contains("auth")
            || l2.contains("middleware")
            || l2.contains("JWT")
            || l2.contains("jwt"),
        "L2 should contain content from requested domain 'task-analysis', got: {l2}"
    );

    // L2 should NOT contain content from unrequested domains.
    assert!(
        !l2.contains("Test suite covering auth middleware"),
        "L2 should not contain content from 'coding/testing/unit-tests' (not requested)"
    );
}

// ===========================================================================
// 10. test_l2_applies_compression
// ===========================================================================

#[test]
fn test_l2_applies_compression() {
    let loader = LayeredContextLoader::new();
    let request = sample_agent_request();
    let store = sample_rlm_store();

    // Get the raw content for requested domains.
    let raw_task = store.read("coding/understanding/task-analysis").unwrap();
    let raw_api = store.read("coding/design/api-design").unwrap();
    let raw_combined_tokens = count_tokens(&raw_task) + count_tokens(&raw_api);

    let l2 = loader.load_l2(&request, &store);
    let l2_tokens = count_tokens(&l2);

    // L2 should be compressed (shorter than raw content).
    assert!(
        l2_tokens < raw_combined_tokens,
        "L2 ({l2_tokens} tokens) should be shorter than raw content ({raw_combined_tokens} tokens) due to compression"
    );
}

// ===========================================================================
// 11. test_l3_none_when_not_triggered
// ===========================================================================

#[test]
fn test_l3_none_when_not_triggered() {
    let loader = LayeredContextLoader::new();
    let identity = sample_identity();
    let patterns = sample_patterns();
    let request = sample_agent_request();
    let store = sample_rlm_store();

    let ctx = loader.load_context(&identity, &patterns, &request, &store, false);

    assert!(
        ctx.l3_deep.is_none(),
        "L3 should be None when trigger_l3 is false"
    );
}

// ===========================================================================
// 12. test_l3_present_when_triggered
// ===========================================================================

#[test]
fn test_l3_present_when_triggered() {
    let loader = LayeredContextLoader::new();
    let identity = sample_identity();
    let patterns = sample_patterns();
    let request = sample_agent_request();
    let store = sample_rlm_store();

    let ctx = loader.load_context(&identity, &patterns, &request, &store, true);

    assert!(
        ctx.l3_deep.is_some(),
        "L3 should be Some when trigger_l3 is true"
    );
    let l3 = ctx.l3_deep.unwrap();
    assert!(
        !l3.is_empty(),
        "L3 content should not be empty when triggered"
    );
}

// ===========================================================================
// 13. test_load_context_combines_all_layers
// ===========================================================================

#[test]
fn test_load_context_combines_all_layers() {
    let loader = LayeredContextLoader::new();
    let identity = sample_identity();
    let patterns = sample_patterns();
    let request = sample_agent_request();
    let store = sample_rlm_store();

    let ctx = loader.load_context(&identity, &patterns, &request, &store, true);

    // All layers should be populated.
    assert!(!ctx.l0_identity.is_empty(), "L0 should be populated");
    assert!(!ctx.l1_patterns.is_empty(), "L1 should be populated");
    assert!(!ctx.l2_on_demand.is_empty(), "L2 should be populated");
    assert!(ctx.l3_deep.is_some(), "L3 should be present when triggered");

    // Total tokens should be the sum of all layer tokens.
    let expected_total = count_tokens(&ctx.l0_identity)
        + count_tokens(&ctx.l1_patterns)
        + count_tokens(&ctx.l2_on_demand)
        + ctx.l3_deep.as_ref().map(|s| count_tokens(s)).unwrap_or(0);

    assert_eq!(
        ctx.total_tokens, expected_total,
        "total_tokens should equal sum of all layer token counts"
    );
}

// ===========================================================================
// 14. test_l2_missing_domains_returns_empty
// ===========================================================================

#[test]
fn test_l2_missing_domains_returns_empty() {
    let loader = LayeredContextLoader::new();
    let request = missing_domains_request();
    let store = sample_rlm_store();

    let l2 = loader.load_l2(&request, &store);

    // Requested domains (frontend/*) do not exist in the store.
    assert!(
        l2.is_empty() || count_tokens(&l2) == 0,
        "L2 should be empty when requested domains are not in the RLM store, got: '{l2}'"
    );
}

// ===========================================================================
// 15. test_wiring_obligations_in_l0
// ===========================================================================

#[test]
fn test_wiring_obligations_in_l0() {
    let loader = LayeredContextLoader::new();
    let identity = sample_identity();
    let l0 = loader.load_l0(&identity);

    assert!(
        l0.contains("coding/implementation/auth-middleware"),
        "L0 should contain wiring obligations when provided, got: {l0}"
    );
}

// ===========================================================================
// 16. test_wiring_obligations_absent_when_none
// ===========================================================================

#[test]
fn test_wiring_obligations_absent_when_none() {
    let loader = LayeredContextLoader::new();
    let identity = identity_no_wiring();
    let l0 = loader.load_l0(&identity);

    // L0 should still be valid and contain identity info.
    assert!(
        l0.contains("sess-xyz-789"),
        "L0 should still contain session_id without wiring, got: {l0}"
    );
    // Should not crash or contain placeholder wiring text.
    assert!(
        !l0.contains("null") && !l0.contains("None"),
        "L0 should not contain literal 'null' or 'None' for absent wiring"
    );
}

// ===========================================================================
// 17. test_load_context_without_l3_has_lower_token_count
// ===========================================================================

#[test]
fn test_load_context_without_l3_has_lower_token_count() {
    let loader = LayeredContextLoader::new();
    let identity = sample_identity();
    let patterns = sample_patterns();
    let request = sample_agent_request();
    let store = sample_rlm_store();

    let ctx_no_l3 = loader.load_context(&identity, &patterns, &request, &store, false);
    let ctx_with_l3 = loader.load_context(&identity, &patterns, &request, &store, true);

    assert!(
        ctx_no_l3.total_tokens <= ctx_with_l3.total_tokens,
        "Context without L3 ({}) should have fewer or equal tokens than with L3 ({})",
        ctx_no_l3.total_tokens,
        ctx_with_l3.total_tokens
    );
}

// ===========================================================================
// 18. test_l2_under_budget
// ===========================================================================

#[test]
fn test_l2_under_budget() {
    let loader = LayeredContextLoader::new();
    let request = sample_agent_request();
    let store = sample_rlm_store();
    let l2 = loader.load_l2(&request, &store);
    let tokens = count_tokens(&l2);

    assert!(
        tokens <= 500,
        "L2 should be under 500 tokens (default budget), got {tokens}"
    );
}

// ===========================================================================
// 19. test_l3_loads_full_semantic_search
// ===========================================================================

#[test]
fn test_l3_loads_full_semantic_search() {
    let loader = LayeredContextLoader::new();
    let store = sample_rlm_store();

    let l3 = loader.load_l3("authentication middleware", &store);

    // L3 performs a full semantic search across all store content.
    assert!(!l3.is_empty(), "L3 should return content for a valid query");
}

// ===========================================================================
// 20. test_load_context_total_tokens_consistent
// ===========================================================================

#[test]
fn test_load_context_total_tokens_consistent() {
    let loader = LayeredContextLoader::new();
    let identity = sample_identity();
    let patterns = sample_patterns();
    let request = sample_agent_request();
    let store = sample_rlm_store();

    let ctx = loader.load_context(&identity, &patterns, &request, &store, false);

    // total_tokens must equal sum of L0+L1+L2 (L3 is None).
    let manual_count = count_tokens(&ctx.l0_identity)
        + count_tokens(&ctx.l1_patterns)
        + count_tokens(&ctx.l2_on_demand);

    assert_eq!(
        ctx.total_tokens, manual_count,
        "total_tokens ({}) should match manual count ({}) when L3 is not triggered",
        ctx.total_tokens, manual_count
    );
}
