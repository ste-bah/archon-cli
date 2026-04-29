//! ReasoningBank 12-mode dispatch tests — proves each enum variant routes to its engine.
//! v0.1.28: wires the 10 previously-unreachable engine modules.

use archon_pipeline::learning::patterns::{CreatePatternParams, PatternStore, TaskType};
use archon_pipeline::learning::reasoning::{
    ModeSelector, ReasoningBank, ReasoningBankConfig, ReasoningBankDeps, ReasoningMode,
    ReasoningRequest,
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

fn build_bank_with_history() -> ReasoningBank {
    let mut bank = ReasoningBank::new(ReasoningBankDeps {
        pattern_store: make_store_with_pattern(),
        causal_memory: None,
        gnn_enhancer: None,
        sona_engine: None,
        config: ReasoningBankConfig::default(),
    });
    // Run a pattern-match query to seed one trajectory
    let _ = bank.reason(&ReasoningRequest {
        query: "seed trajectory".to_string(),
        query_embedding: None,
        mode: Some(ReasoningMode::PatternMatch),
        task_type: Some(TaskType::Coding),
        max_results: Some(5),
        confidence_threshold: Some(0.0),
        context: None,
    });
    bank
}

// ---------------------------------------------------------------------------
// Per-mode dispatch tests — one per spec mode
// ---------------------------------------------------------------------------

#[test]
fn deductive_mode_dispatches_to_engine() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "Therefore, X implies Y".to_string(),
        mode: Some(ReasoningMode::Deductive),
        context: Some(vec![
            "if: it rains then the ground is wet".to_string(),
            "premise: it rains".to_string(),
        ]),
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Deductive);
    assert!(
        response.overall_confidence > 0.0,
        "Deductive engine should produce non-zero confidence on a logical query"
    );
    assert!(
        response
            .context_metadata
            .iter()
            .any(|(_, v)| v.contains("deductive")),
        "engine_name should be threaded through to context_metadata"
    );
}

#[test]
fn inductive_mode_dispatches_to_engine() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "generalize from these examples".to_string(),
        mode: Some(ReasoningMode::Inductive),
        context: Some(vec![
            "example:bird|wings,feathers,beak".to_string(),
            "example:bird|wings,feathers,talons".to_string(),
            "example:bird|wings,beak,talons".to_string(),
        ]),
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Inductive);
    assert!(
        response.overall_confidence > 0.0,
        "Inductive engine should produce non-zero confidence with examples"
    );
    assert!(
        response
            .context_metadata
            .iter()
            .any(|(_, v)| v.contains("inductive"))
    );
}

#[test]
fn abductive_mode_dispatches_to_engine() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "explain why the server crashed".to_string(),
        mode: Some(ReasoningMode::Abductive),
        context: Some(vec![
            "high memory usage observed".to_string(),
            "disk full alert triggered".to_string(),
            "OOM killer active".to_string(),
        ]),
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Abductive);
    assert!(
        response.overall_confidence > 0.0,
        "Abductive engine should produce non-zero confidence"
    );
    assert!(
        response
            .context_metadata
            .iter()
            .any(|(_, v)| v.contains("abductive"))
    );
}

#[test]
fn analogical_mode_dispatches_to_engine() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "how is a CPU similar to a brain".to_string(),
        mode: Some(ReasoningMode::Analogical),
        context: Some(vec![
            "CPU processes instructions".to_string(),
            "brain processes signals".to_string(),
        ]),
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Analogical);
    assert!(
        response.overall_confidence > 0.0,
        "Analogical engine should produce non-zero confidence"
    );
    assert!(
        response
            .context_metadata
            .iter()
            .any(|(_, v)| v.contains("analogical"))
    );
}

#[test]
fn adversarial_mode_dispatches_to_engine() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "argue against: Rust is the best language".to_string(),
        mode: Some(ReasoningMode::Adversarial),
        context: Some(vec![
            "Rust has a steep learning curve".to_string(),
            "Garbage collected languages are simpler".to_string(),
        ]),
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Adversarial);
    assert!(
        response.overall_confidence > 0.0,
        "Adversarial engine should produce non-zero confidence"
    );
    assert!(
        response
            .context_metadata
            .iter()
            .any(|(_, v)| v.contains("adversarial"))
    );
}

#[test]
fn counterfactual_mode_dispatches_to_engine() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "what if we had used a cache".to_string(),
        mode: Some(ReasoningMode::Counterfactual),
        context: Some(vec![
            "current latency is 200ms".to_string(),
            "database queries dominate".to_string(),
            "cache hit rate would be 80%".to_string(),
        ]),
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Counterfactual);
    assert!(
        response.overall_confidence > 0.0,
        "Counterfactual engine should produce non-zero confidence"
    );
    assert!(
        response
            .context_metadata
            .iter()
            .any(|(_, v)| v.contains("counterfactual"))
    );
}

#[test]
fn temporal_mode_dispatches_to_engine() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "analyze deployment timeline".to_string(),
        mode: Some(ReasoningMode::Temporal),
        context: Some(vec![
            "event:build:0:10".to_string(),
            "event:test:10:30".to_string(),
            "event:deploy:30:35".to_string(),
            "event:monitor:35:60".to_string(),
        ]),
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Temporal);
    assert!(
        response.overall_confidence > 0.0,
        "Temporal engine should produce non-zero confidence"
    );
    assert!(
        response
            .context_metadata
            .iter()
            .any(|(_, v)| v.contains("temporal"))
    );
}

#[test]
fn constraint_mode_dispatches_to_engine() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "assign tasks to workers".to_string(),
        mode: Some(ReasoningMode::Constraint),
        context: Some(vec![
            "var:worker1:task_a,task_b".to_string(),
            "var:worker2:task_a,task_c".to_string(),
            "constraint:worker1!=worker2".to_string(),
        ]),
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Constraint);
    assert!(
        response.overall_confidence > 0.0,
        "Constraint engine should produce non-zero confidence"
    );
    assert!(
        response
            .context_metadata
            .iter()
            .any(|(_, v)| v.contains("constraint"))
    );
}

#[test]
fn decomposition_mode_dispatches_to_engine() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "build a web application".to_string(),
        mode: Some(ReasoningMode::Decomposition),
        context: Some(vec![
            "need frontend".to_string(),
            "need backend API".to_string(),
            "need database".to_string(),
            "frontend depends on API".to_string(),
            "API depends on database".to_string(),
        ]),
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Decomposition);
    assert!(
        response.overall_confidence > 0.0,
        "Decomposition engine should produce non-zero confidence"
    );
    assert!(
        response
            .context_metadata
            .iter()
            .any(|(_, v)| v.contains("decomposition"))
    );
}

#[test]
fn first_principles_mode_dispatches_to_engine() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "why do we need automated testing".to_string(),
        mode: Some(ReasoningMode::FirstPrinciples),
        context: Some(vec![
            "axiom: software has bugs".to_string(),
            "axiom: manual testing is slow".to_string(),
            "axiom: regressions occur on change".to_string(),
        ]),
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::FirstPrinciples);
    assert!(
        response.overall_confidence > 0.0,
        "FirstPrinciples engine should produce non-zero confidence"
    );
    assert!(
        response
            .context_metadata
            .iter()
            .any(|(_, v)| v.contains("first_principles"))
    );
}

#[test]
fn causal_mode_dispatches_to_causal_memory() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "the cause of the failure".to_string(),
        mode: Some(ReasoningMode::Causal),
        context: None,
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Causal);
    // Causal mode may return 0.0 confidence when CausalMemory is None,
    // but it must still dispatch to the correct mode.
    assert!(
        response
            .provenance
            .iter()
            .any(|p| p.mode == ReasoningMode::Causal)
    );
}

#[test]
fn contextual_mode_dispatches_to_contextual() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "in the context of distributed systems".to_string(),
        query_embedding: Some(vec![1.0, 0.0, 0.0]),
        mode: Some(ReasoningMode::Contextual),
        context: None,
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Contextual);
    // Contextual mode uses GNN-enhanced similarity; may produce patterns
    // if embeddings are provided and threshold is low enough
    assert!(
        response
            .provenance
            .iter()
            .any(|p| p.mode == ReasoningMode::Contextual)
    );
}

// ---------------------------------------------------------------------------
// Auto-mode selection table test
// ---------------------------------------------------------------------------

#[test]
fn auto_mode_selection_picks_correct_mode_for_each_query_pattern() {
    let cases: Vec<(&str, ReasoningMode)> = vec![
        ("therefore X implies Y", ReasoningMode::Deductive),
        (
            "there is a general rule that always holds",
            ReasoningMode::Inductive,
        ),
        ("why did the system crash", ReasoningMode::Abductive),
        (
            "this is similar to the auth flow",
            ReasoningMode::Analogical,
        ),
        (
            "how could this break the system",
            ReasoningMode::Adversarial,
        ),
        (
            "what if we had used postgres",
            ReasoningMode::Counterfactual,
        ),
        (
            "when did this happen, before deploy?",
            ReasoningMode::Temporal,
        ),
        (
            "must satisfy the constraint that x > 0",
            ReasoningMode::Constraint,
        ),
        (
            "break down the steps to refactor",
            ReasoningMode::Decomposition,
        ),
        (
            "from first principles, derive the answer",
            ReasoningMode::FirstPrinciples,
        ),
        ("the cause of the failure is unknown", ReasoningMode::Causal),
        (
            "in the context of distributed systems",
            ReasoningMode::Contextual,
        ),
    ];
    for (query, expected) in cases {
        let selected = ModeSelector::select(query);
        assert_eq!(
            selected, expected,
            "Query '{}' should select {:?}, got {:?}",
            query, expected, selected
        );
    }
}

// ---------------------------------------------------------------------------
// Hybrid mode aggregates from multiple sources
// ---------------------------------------------------------------------------

#[test]
fn hybrid_mode_aggregates_results_from_multiple_sources() {
    let mut bank = build_bank_with_history();
    let response = bank.reason(&ReasoningRequest {
        query: "general analysis of system architecture".to_string(),
        query_embedding: Some(vec![1.0, 0.0, 0.0]),
        mode: Some(ReasoningMode::Hybrid),
        task_type: Some(TaskType::Coding),
        context: None,
        ..Default::default()
    });
    assert_eq!(response.mode_used, ReasoningMode::Hybrid);
    // Hybrid should produce provenance from at least 3 different sources
    // (pattern_store, causal_memory, raw_embeddings)
    let unique_sources: std::collections::HashSet<_> = response
        .provenance
        .iter()
        .map(|p| p.source.clone())
        .collect();
    assert!(
        unique_sources.len() >= 3,
        "Hybrid should aggregate at least 3 different sources, got {} from {:?}",
        unique_sources.len(),
        unique_sources
    );
}
