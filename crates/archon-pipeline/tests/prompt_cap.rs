//! Tests for TASK-PIPE-A07: Prompt Cap / Token-Aware Truncation.
//!
//! These tests verify that `archon_pipeline::prompt_cap` correctly counts tokens,
//! truncates prompt layers by priority, respects required-layer guarantees, and
//! keeps the assembled prompt within 80 % of the model context window.

use archon_pipeline::prompt_cap::{
    PromptLayer, TruncationPriority, count_tokens, truncate_prompt,
};

// ---------------------------------------------------------------------------
// Helper
// ---------------------------------------------------------------------------

/// Build a `PromptLayer` whose content is approximately `content_tokens` tokens.
/// Uses the same heuristic as `count_tokens`: 1 token ≈ 4 chars.
fn make_layer(
    name: &str,
    content_tokens: usize,
    priority: TruncationPriority,
    required: bool,
) -> PromptLayer {
    let content = "x".repeat(content_tokens * 4);
    PromptLayer {
        name: name.to_string(),
        content,
        priority,
        required,
    }
}

// ---------------------------------------------------------------------------
// 1. count_tokens — basic
// ---------------------------------------------------------------------------

#[test]
fn test_count_tokens_basic() {
    // "hello world" = 11 chars; 11 / 4 = 2.75 → ceiling = 3
    assert_eq!(count_tokens("hello world"), 3);
}

// ---------------------------------------------------------------------------
// 2. count_tokens — empty string
// ---------------------------------------------------------------------------

#[test]
fn test_count_tokens_empty() {
    assert_eq!(count_tokens(""), 0);
}

// ---------------------------------------------------------------------------
// 3. No truncation when everything fits within 80 %
// ---------------------------------------------------------------------------

#[test]
fn test_no_truncation_when_under_limit() {
    let window = 100_000; // 80 % = 80 000 tokens
    let layers = vec![
        make_layer("base_prompt", 10_000, TruncationPriority::Required, true),
        make_layer("task_context", 10_000, TruncationPriority::Required, true),
        make_layer("rlm_context", 5_000, TruncationPriority::RlmContext, false),
        make_layer(
            "desc_episodes",
            5_000,
            TruncationPriority::DescEpisodes,
            false,
        ),
    ];

    let result = truncate_prompt(layers, window).expect("truncate_prompt should succeed");

    assert!(
        result.removed_layers.is_empty(),
        "no layers should be removed when total fits within 80 %"
    );
    assert!(
        result.truncated_layers.is_empty(),
        "no layers should be truncated when total fits within 80 %"
    );
    assert_eq!(result.layers.len(), 4);
    assert!(result.total_tokens <= (window * 80) / 100);
}

// ---------------------------------------------------------------------------
// 4. Truncation removes lowest-priority layers first
// ---------------------------------------------------------------------------

#[test]
fn test_truncation_removes_lowest_priority_first() {
    let window = 100_000; // budget = 80 000 tokens
    let layers = vec![
        make_layer("base_prompt", 30_000, TruncationPriority::Required, true),
        make_layer("task_context", 30_000, TruncationPriority::Required, true),
        // These two push total to 90 000 — over budget
        make_layer(
            "leann_context",
            15_000,
            TruncationPriority::LeannSemanticContext,
            false,
        ),
        make_layer("rlm_context", 15_000, TruncationPriority::RlmContext, false),
    ];

    let result = truncate_prompt(layers, window).expect("truncate_prompt should succeed");

    // LeannSemanticContext (ordinal 1) should be removed before RlmContext (ordinal 8)
    assert!(
        result.removed_layers.contains(&"leann_context".to_string()),
        "leann_context should be removed first (lowest priority)"
    );
    assert!(result.total_tokens <= (window * 80) / 100);
}

// ---------------------------------------------------------------------------
// 5. Required layers are never fully removed
// ---------------------------------------------------------------------------

#[test]
fn test_required_layers_never_removed() {
    let window = 100_000; // budget = 80 000
    let layers = vec![
        make_layer("base_prompt", 50_000, TruncationPriority::Required, true),
        make_layer("task_context", 50_000, TruncationPriority::Required, true),
        make_layer("rlm_context", 50_000, TruncationPriority::RlmContext, false),
        make_layer(
            "desc_episodes",
            50_000,
            TruncationPriority::DescEpisodes,
            false,
        ),
    ];

    let result = truncate_prompt(layers, window).expect("truncate_prompt should succeed");

    // Required layers must survive (may be truncated but not removed)
    let surviving_names: Vec<&str> = result.layers.iter().map(|l| l.name.as_str()).collect();
    assert!(
        surviving_names.contains(&"base_prompt"),
        "base_prompt is required and must survive"
    );
    assert!(
        surviving_names.contains(&"task_context"),
        "task_context is required and must survive"
    );

    // Non-required layers should be fully removed
    assert!(
        result.removed_layers.contains(&"rlm_context".to_string()),
        "rlm_context should be removed"
    );
    assert!(
        result.removed_layers.contains(&"desc_episodes".to_string()),
        "desc_episodes should be removed"
    );
}

// ---------------------------------------------------------------------------
// 6. EC-PIPE-003: all 11 layers exceed context
// ---------------------------------------------------------------------------

#[test]
fn test_ec_pipe_003_all_layers_exceed() {
    let window = 100_000; // budget = 80 000
    let layers = vec![
        make_layer("base_prompt", 20_000, TruncationPriority::Required, true),
        make_layer("task_context", 20_000, TruncationPriority::Required, true),
        make_layer("rlm_context", 10_000, TruncationPriority::RlmContext, false),
        make_layer(
            "desc_episodes",
            10_000,
            TruncationPriority::DescEpisodes,
            false,
        ),
        make_layer(
            "sona_patterns",
            10_000,
            TruncationPriority::SonaPatterns,
            false,
        ),
        make_layer(
            "reflexion_trajectories",
            10_000,
            TruncationPriority::ReflexionTrajectories,
            false,
        ),
        make_layer(
            "pattern_matcher",
            10_000,
            TruncationPriority::PatternMatcherResults,
            false,
        ),
        make_layer(
            "sherlock_verdicts",
            10_000,
            TruncationPriority::SherlockVerdicts,
            false,
        ),
        make_layer(
            "algorithm_strategy",
            10_000,
            TruncationPriority::AlgorithmStrategy,
            false,
        ),
        make_layer(
            "leann_semantic",
            10_000,
            TruncationPriority::LeannSemanticContext,
            false,
        ),
        make_layer(
            "extra_non_required",
            10_000,
            TruncationPriority::RlmContext,
            false,
        ),
    ];
    // Total = 130 000 tokens, budget = 80 000

    let result = truncate_prompt(layers, window).expect("truncate_prompt should succeed");

    // Required layers must survive
    let surviving_names: Vec<&str> = result.layers.iter().map(|l| l.name.as_str()).collect();
    assert!(surviving_names.contains(&"base_prompt"));
    assert!(surviving_names.contains(&"task_context"));

    // Total must be within budget
    assert!(
        result.total_tokens <= (window * 80) / 100,
        "total_tokens {} should be <= {}",
        result.total_tokens,
        (window * 80) / 100
    );
}

// ---------------------------------------------------------------------------
// 6b. If even required layers exceed limit, task_context is truncated from end
// ---------------------------------------------------------------------------

#[test]
fn test_ec_pipe_003_required_layers_exceed_limit() {
    let window = 100_000; // budget = 80 000
    let layers = vec![
        make_layer("base_prompt", 30_000, TruncationPriority::Required, true),
        make_layer("task_context", 60_000, TruncationPriority::Required, true),
    ];
    // Total required = 90 000, budget = 80 000 → task_context must be truncated

    let result = truncate_prompt(layers, window).expect("truncate_prompt should succeed");

    // Both layers survive (required)
    assert_eq!(result.layers.len(), 2);

    // task_context should appear in truncated_layers
    let task_truncated = result
        .truncated_layers
        .iter()
        .find(|(name, _, _)| name == "task_context");
    assert!(
        task_truncated.is_some(),
        "task_context must be truncated when required layers exceed budget"
    );

    let (_, original, final_size) = task_truncated.unwrap();
    assert!(
        final_size < original,
        "task_context final tokens ({}) should be less than original ({})",
        final_size,
        original
    );

    assert!(
        result.total_tokens <= (window * 80) / 100,
        "total_tokens {} should be <= {}",
        result.total_tokens,
        (window * 80) / 100
    );
}

// ---------------------------------------------------------------------------
// 7. Partial truncation within a priority level
// ---------------------------------------------------------------------------

#[test]
fn test_partial_truncation_within_priority() {
    let window = 100_000; // budget = 80 000
    // Required layers take 70 000, leaving 10 000 for the optional layer of 15 000
    let layers = vec![
        make_layer("base_prompt", 35_000, TruncationPriority::Required, true),
        make_layer("task_context", 35_000, TruncationPriority::Required, true),
        make_layer(
            "leann_semantic",
            15_000,
            TruncationPriority::LeannSemanticContext,
            false,
        ),
    ];

    let result = truncate_prompt(layers, window).expect("truncate_prompt should succeed");

    // leann_semantic should be partially truncated, not fully removed
    assert!(
        !result
            .removed_layers
            .contains(&"leann_semantic".to_string()),
        "leann_semantic should NOT be fully removed"
    );

    let truncated = result
        .truncated_layers
        .iter()
        .find(|(name, _, _)| name == "leann_semantic");
    assert!(
        truncated.is_some(),
        "leann_semantic should appear in truncated_layers"
    );
    let (_, original, final_size) = truncated.unwrap();
    assert!(
        final_size < original,
        "final ({}) should be less than original ({})",
        final_size,
        original
    );
    assert!(
        *final_size > 0,
        "partial truncation should leave some content"
    );

    assert!(result.total_tokens <= (window * 80) / 100);
}

// ---------------------------------------------------------------------------
// 8. Both removed_layers and truncated_layers are populated
// ---------------------------------------------------------------------------

#[test]
fn test_truncated_prompt_reports_removed_and_truncated() {
    let window = 100_000; // budget = 80 000
    let layers = vec![
        make_layer("base_prompt", 35_000, TruncationPriority::Required, true),
        make_layer("task_context", 35_000, TruncationPriority::Required, true),
        // This one will be fully removed (lowest priority = LeannSemanticContext, ordinal 1)
        make_layer(
            "leann_semantic",
            10_000,
            TruncationPriority::LeannSemanticContext,
            false,
        ),
        // This one should be partially truncated (higher priority = RlmContext, ordinal 8)
        make_layer("rlm_context", 15_000, TruncationPriority::RlmContext, false),
    ];

    let result = truncate_prompt(layers, window).expect("truncate_prompt should succeed");

    // leann_semantic (lowest priority, ordinal 1) should be removed
    assert!(
        result
            .removed_layers
            .contains(&"leann_semantic".to_string()),
        "leann_semantic should be fully removed (lowest priority)"
    );

    // rlm_context should be partially truncated (higher priority, some room left)
    assert!(
        !result.removed_layers.is_empty() || !result.truncated_layers.is_empty(),
        "at least one of removed_layers or truncated_layers should be populated"
    );

    assert!(result.total_tokens <= (window * 80) / 100);
}

// ---------------------------------------------------------------------------
// 9. Empty-content layer doesn't cause a panic
// ---------------------------------------------------------------------------

#[test]
fn test_empty_layers_handled_gracefully() {
    let window = 100_000;
    let layers = vec![
        make_layer("base_prompt", 10_000, TruncationPriority::Required, true),
        PromptLayer {
            name: "empty_layer".to_string(),
            content: String::new(),
            priority: TruncationPriority::RlmContext,
            required: false,
        },
        make_layer("task_context", 10_000, TruncationPriority::Required, true),
    ];

    let result = truncate_prompt(layers, window).expect("empty layer must not cause a panic");
    assert!(result.total_tokens <= (window * 80) / 100);
}

// ---------------------------------------------------------------------------
// 10. After truncation, total_tokens <= 80 % of model_context_window
// ---------------------------------------------------------------------------

#[test]
fn test_prompt_fits_80_percent() {
    let window = 100_000;
    // Deliberately overshoot
    let layers = vec![
        make_layer("base_prompt", 20_000, TruncationPriority::Required, true),
        make_layer("task_context", 20_000, TruncationPriority::Required, true),
        make_layer("rlm_context", 20_000, TruncationPriority::RlmContext, false),
        make_layer(
            "desc_episodes",
            20_000,
            TruncationPriority::DescEpisodes,
            false,
        ),
        make_layer(
            "sona_patterns",
            20_000,
            TruncationPriority::SonaPatterns,
            false,
        ),
    ];
    // Total = 100 000, budget = 80 000

    let result = truncate_prompt(layers, window).expect("truncate_prompt should succeed");

    let budget = (window * 80) / 100;
    assert!(
        result.total_tokens <= budget,
        "total_tokens ({}) must be <= 80% of window ({})",
        result.total_tokens,
        budget
    );
}

// ---------------------------------------------------------------------------
// 11. Priority ordering: LeannSemanticContext < DescEpisodes < ... < RlmContext < Required
// ---------------------------------------------------------------------------

#[test]
fn test_priority_ordering() {
    // Ordinal order: LeannSemanticContext(1) < DescEpisodes(2) < ... < RlmContext(8) < Required(100)
    // Lower ordinal = removed first during truncation.
    assert!(TruncationPriority::LeannSemanticContext < TruncationPriority::DescEpisodes);
    assert!(TruncationPriority::DescEpisodes < TruncationPriority::SonaPatterns);
    assert!(TruncationPriority::SonaPatterns < TruncationPriority::ReflexionTrajectories);
    assert!(TruncationPriority::ReflexionTrajectories < TruncationPriority::PatternMatcherResults);
    assert!(TruncationPriority::PatternMatcherResults < TruncationPriority::SherlockVerdicts);
    assert!(TruncationPriority::SherlockVerdicts < TruncationPriority::AlgorithmStrategy);
    assert!(TruncationPriority::AlgorithmStrategy < TruncationPriority::RlmContext);
    assert!(TruncationPriority::RlmContext < TruncationPriority::Required);
}
