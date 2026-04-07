//! Tests for ReasoningBank — 4 modes, ModeSelector, TrajectoryTracker.
//! Covers REQ-LEARN-005.

use archon_pipeline::learning::patterns::{CreatePatternParams, PatternStore, TaskType};
use archon_pipeline::learning::reasoning::{
    ModeSelector, ReasoningBank, ReasoningBankConfig, ReasoningBankDeps, ReasoningMode,
    ReasoningRequest, TrajectoryTracker,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_store_with_pattern() -> PatternStore {
    let mut store = PatternStore::new();
    store
        .create_pattern(CreatePatternParams {
            task_type: TaskType::Coding,
            template: "use a loop".to_string(),
            embedding: vec![1.0, 0.0, 0.0],
            initial_success_rate: 0.8,
        })
        .expect("create pattern");
    store
}

fn make_bank(store: PatternStore) -> ReasoningBank {
    ReasoningBank::new(ReasoningBankDeps {
        pattern_store: store,
        causal_memory: None,
        gnn_enhancer: None,
        sona_engine: None,
        config: ReasoningBankConfig::default(),
    })
}

// ---------------------------------------------------------------------------
// Test 1: PatternMatch mode returns results from PatternMatcher
// ---------------------------------------------------------------------------

#[test]
fn test_pattern_match_mode_returns_results() {
    let store = make_store_with_pattern();
    let mut bank = make_bank(store);

    let req = ReasoningRequest {
        query: "how to iterate".to_string(),
        query_embedding: Some(vec![1.0, 0.0, 0.0]),
        mode: Some(ReasoningMode::PatternMatch),
        task_type: Some(TaskType::Coding),
        max_results: Some(10),
        confidence_threshold: Some(0.0),
    };

    let resp = bank.reason(&req);
    assert_eq!(resp.mode_used, ReasoningMode::PatternMatch);
    assert!(!resp.patterns.is_empty(), "PatternMatch should return at least one result");
    assert!(resp.overall_confidence > 0.0);
}

// ---------------------------------------------------------------------------
// Test 2: CausalInference mode returns empty results with warning (stubbed)
// ---------------------------------------------------------------------------

#[test]
fn test_causal_inference_mode_is_stubbed() {
    let mut bank = make_bank(PatternStore::new());

    let req = ReasoningRequest {
        query: "why did the build fail".to_string(),
        query_embedding: None,
        mode: Some(ReasoningMode::CausalInference),
        task_type: None,
        max_results: None,
        confidence_threshold: None,
    };

    let resp = bank.reason(&req);
    assert_eq!(resp.mode_used, ReasoningMode::CausalInference);
    assert!(resp.patterns.is_empty(), "CausalInference stub returns no patterns");
    assert!(resp.inferences.is_empty(), "CausalInference stub returns no inferences");
    assert_eq!(resp.overall_confidence, 0.0);
}

// ---------------------------------------------------------------------------
// Test 3: Contextual mode returns vector similarity results
// ---------------------------------------------------------------------------

#[test]
fn test_contextual_mode_returns_similarity_results() {
    let store = make_store_with_pattern();
    let mut bank = make_bank(store);

    let req = ReasoningRequest {
        query: "similar to a loop".to_string(),
        query_embedding: Some(vec![1.0, 0.0, 0.0]),
        mode: Some(ReasoningMode::Contextual),
        task_type: None,
        max_results: Some(10),
        confidence_threshold: Some(0.0),
    };

    let resp = bank.reason(&req);
    assert_eq!(resp.mode_used, ReasoningMode::Contextual);
    assert!(!resp.patterns.is_empty(), "Contextual mode should return similarity results");
    assert!(resp.overall_confidence > 0.0);
}

// ---------------------------------------------------------------------------
// Test 4: Hybrid mode combines results with configurable weights (0.3 + 0.3 + 0.4)
// ---------------------------------------------------------------------------

#[test]
fn test_hybrid_mode_combines_results() {
    let store = make_store_with_pattern();
    let mut bank = make_bank(store);

    let req = ReasoningRequest {
        query: "generic query about coding".to_string(),
        query_embedding: Some(vec![1.0, 0.0, 0.0]),
        mode: Some(ReasoningMode::Hybrid),
        task_type: Some(TaskType::Coding),
        max_results: Some(10),
        confidence_threshold: Some(0.0),
    };

    let resp = bank.reason(&req);
    assert_eq!(resp.mode_used, ReasoningMode::Hybrid);
    // Provenance should reflect all 3 sources
    assert_eq!(resp.provenance.len(), 3);
}

#[test]
fn test_hybrid_weights_sum_to_one() {
    let config = ReasoningBankConfig::default();
    let total = config.pattern_weight + config.causal_weight + config.contextual_weight;
    // Allow floating-point tolerance
    assert!((total - 1.0).abs() < 1e-9, "weights must sum to 1.0, got {}", total);
}

// ---------------------------------------------------------------------------
// Test 5: ModeSelector: "why" queries -> CausalInference
// ---------------------------------------------------------------------------

#[test]
fn test_mode_selector_why_query() {
    let mode = ModeSelector::select("why is the server crashing");
    assert_eq!(mode, ReasoningMode::CausalInference);
}

// ---------------------------------------------------------------------------
// Test 6: ModeSelector: "similar to" queries -> Contextual
// ---------------------------------------------------------------------------

#[test]
fn test_mode_selector_similar_to_query() {
    let mode = ModeSelector::select("find code similar to this function");
    assert_eq!(mode, ReasoningMode::Contextual);
}

// ---------------------------------------------------------------------------
// Test 7: ModeSelector: "how to" queries -> PatternMatch
// ---------------------------------------------------------------------------

#[test]
fn test_mode_selector_how_to_query() {
    let mode = ModeSelector::select("how to implement a binary search");
    assert_eq!(mode, ReasoningMode::PatternMatch);
}

// ---------------------------------------------------------------------------
// Test 8: ModeSelector: generic queries -> Hybrid
// ---------------------------------------------------------------------------

#[test]
fn test_mode_selector_generic_query() {
    let mode = ModeSelector::select("refactor this module");
    assert_eq!(mode, ReasoningMode::Hybrid);
}

// ---------------------------------------------------------------------------
// Test 9: TrajectoryTracker records execution path
// ---------------------------------------------------------------------------

#[test]
fn test_trajectory_tracker_records_path() {
    let store = make_store_with_pattern();
    let mut bank = make_bank(store);

    let req = ReasoningRequest {
        query: "how to sort a list".to_string(),
        query_embedding: None,
        mode: Some(ReasoningMode::PatternMatch),
        task_type: Some(TaskType::Coding),
        max_results: None,
        confidence_threshold: Some(0.0),
    };

    bank.reason(&req);
    let trajectories = bank.trajectories();
    assert_eq!(trajectories.len(), 1, "should have recorded one trajectory");
    let traj = &trajectories[0];
    assert_eq!(traj.mode, ReasoningMode::PatternMatch);
    assert!(!traj.trajectory_id.is_empty());
    assert!(!traj.steps.is_empty());
}

// ---------------------------------------------------------------------------
// Test 10: Default config matches TypeScript defaults
// ---------------------------------------------------------------------------

#[test]
fn test_default_config_matches_typescript_defaults() {
    let config = ReasoningBankConfig::default();
    assert!((config.pattern_weight - 0.3).abs() < 1e-9);
    assert!((config.causal_weight - 0.3).abs() < 1e-9);
    assert!((config.contextual_weight - 0.4).abs() < 1e-9);
    assert_eq!(config.default_max_results, 10);
    assert!((config.default_confidence_threshold - 0.7).abs() < 1e-9);
    assert!((config.default_min_l_score - 0.5).abs() < 1e-9);
    assert!(config.enable_trajectory_tracking);
    assert!(config.enable_auto_mode_selection);
}

// ---------------------------------------------------------------------------
// Test 11: GNNEnhancer is None initially; contextual mode works without it
// ---------------------------------------------------------------------------

#[test]
fn test_contextual_mode_works_without_gnn_enhancer() {
    let store = make_store_with_pattern();
    let mut bank = ReasoningBank::new(ReasoningBankDeps {
        pattern_store: store,
        causal_memory: None,
        gnn_enhancer: None, // explicitly None
        sona_engine: None,
        config: ReasoningBankConfig::default(),
    });

    let req = ReasoningRequest {
        query: "similar to a loop".to_string(),
        query_embedding: Some(vec![1.0, 0.0, 0.0]),
        mode: Some(ReasoningMode::Contextual),
        task_type: None,
        max_results: Some(10),
        confidence_threshold: Some(0.0),
    };

    // Must not panic
    let resp = bank.reason(&req);
    assert_eq!(resp.mode_used, ReasoningMode::Contextual);
    // Provenance should say raw_embeddings, not gnn_enhanced
    assert!(resp.provenance.iter().any(|p| p.source == "raw_embeddings"));
}

// ---------------------------------------------------------------------------
// Test 12: Dependency injection via ReasoningBankDeps struct
// ---------------------------------------------------------------------------

#[test]
fn test_dependency_injection_via_deps_struct() {
    let custom_config = ReasoningBankConfig {
        pattern_weight: 0.5,
        causal_weight: 0.2,
        contextual_weight: 0.3,
        default_max_results: 5,
        default_confidence_threshold: 0.6,
        default_min_l_score: 0.4,
        enable_trajectory_tracking: false,
        enable_auto_mode_selection: false,
    };

    let mut bank = ReasoningBank::new(ReasoningBankDeps {
        pattern_store: PatternStore::new(),
        causal_memory: None,
        gnn_enhancer: None,
        sona_engine: None,
        config: custom_config,
    });

    let req = ReasoningRequest {
        query: "anything".to_string(),
        query_embedding: None,
        mode: Some(ReasoningMode::CausalInference),
        task_type: None,
        max_results: None,
        confidence_threshold: None,
    };

    bank.reason(&req);

    // Tracking disabled → no trajectories recorded
    assert_eq!(bank.trajectories().len(), 0);
}

// ---------------------------------------------------------------------------
// Test 13: TrajectoryTracker standalone — record produces valid fields
// ---------------------------------------------------------------------------

#[test]
fn test_trajectory_tracker_standalone() {
    use archon_pipeline::learning::reasoning::ReasoningResponse;

    let dummy_resp = ReasoningResponse {
        mode_used: ReasoningMode::Hybrid,
        patterns: vec![],
        inferences: vec![],
        overall_confidence: 0.42,
        provenance: vec![],
        trajectory_id: None,
    };

    let record = TrajectoryTracker::record("test query", ReasoningMode::Hybrid, &dummy_resp);
    assert!(!record.trajectory_id.is_empty());
    assert_eq!(record.mode, ReasoningMode::Hybrid);
    assert_eq!(record.query, "test query");
    assert!(!record.steps.is_empty());
    assert_eq!(record.result_count, 0);
    assert!((record.confidence - 0.42).abs() < 1e-9);
}
