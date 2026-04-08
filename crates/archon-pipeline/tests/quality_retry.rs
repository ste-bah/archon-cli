//! Tests for TASK-PIPE-A06: Quality Gate Retry Logic
//!
//! These tests verify:
//! - Accepted on first attempt when score meets threshold
//! - Retry improves score on second attempt
//! - Critical agent fails after max retries exhausted
//! - Non-critical agent skipped after max retries exhausted
//! - Empty output triggers retry with feedback containing "empty"
//! - Zero score triggers retry with comprehensive feedback
//! - Feedback contains dimension scores for low-scoring dimensions
//! - Exactly max_retries+1 total attempts honored
//! - Default QualityRetryConfig values

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::Result;

use archon_pipeline::retry::{
    QualityRetryConfig, QualityRetryResult, build_quality_feedback, retry_on_quality,
};
use archon_pipeline::runner::{AgentInfo, QualityScore, ToolAccessLevel};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Creates an AgentInfo with the given key, criticality, and quality threshold.
fn make_agent(key: &str, critical: bool, threshold: f64) -> AgentInfo {
    AgentInfo {
        key: key.to_string(),
        display_name: key.to_string(),
        model: "test".to_string(),
        phase: 1,
        critical,
        quality_threshold: threshold,
        tool_access_level: ToolAccessLevel::ReadOnly,
    }
}

/// Creates a closure that returns predetermined scores per attempt.
///
/// Each call increments the counter and returns the score at that index.
/// If the index exceeds the provided list, returns 0.0.
fn make_scoring_agent(
    scores: Vec<f64>,
    counter: Arc<AtomicU32>,
) -> impl Fn(String) -> Pin<Box<dyn Future<Output = Result<(String, QualityScore)>>>> {
    move |_feedback| {
        let attempt = counter.fetch_add(1, Ordering::SeqCst) as usize;
        let score = scores.get(attempt).copied().unwrap_or(0.0);
        Box::pin(async move {
            Ok((
                format!("output attempt {}", attempt + 1),
                QualityScore {
                    overall: score,
                    dimensions: HashMap::new(),
                },
            ))
        })
    }
}

/// Creates a closure that captures the feedback strings it receives, in addition
/// to returning predetermined scores.
fn make_scoring_agent_with_feedback_capture(
    scores: Vec<f64>,
    counter: Arc<AtomicU32>,
    feedbacks: Arc<std::sync::Mutex<Vec<String>>>,
) -> impl Fn(String) -> Pin<Box<dyn Future<Output = Result<(String, QualityScore)>>>> {
    move |feedback| {
        let attempt = counter.fetch_add(1, Ordering::SeqCst) as usize;
        let score = scores.get(attempt).copied().unwrap_or(0.0);
        feedbacks.lock().unwrap().push(feedback);
        Box::pin(async move {
            Ok((
                format!("output attempt {}", attempt + 1),
                QualityScore {
                    overall: score,
                    dimensions: HashMap::new(),
                },
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// 1. Score 0.85 >= threshold 0.6 — returns Accepted with attempt=1, no retries.
#[tokio::test]
async fn test_accepted_on_first_attempt() {
    let config = QualityRetryConfig {
        max_retries: 3,
        quality_threshold: 0.6,
    };
    let agent = make_agent("analyzer", true, 0.6);
    let initial_output = "good output";
    let initial_score = QualityScore {
        overall: 0.85,
        dimensions: HashMap::new(),
    };

    // The run_agent closure should never be called since initial score is acceptable.
    let counter = Arc::new(AtomicU32::new(0));
    let run_agent = make_scoring_agent(vec![], counter.clone());

    let result = retry_on_quality(&config, &agent, initial_output, &initial_score, run_agent)
        .await
        .expect("should not error");

    match result {
        QualityRetryResult::Accepted {
            output,
            score,
            attempt,
        } => {
            assert_eq!(attempt, 1, "should be accepted on first attempt");
            assert!((score.overall - 0.85).abs() < f64::EPSILON);
            assert_eq!(output, "good output");
        }
        other => panic!("expected Accepted, got {:?}", variant_name(&other)),
    }

    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "run_agent should never be called when initial score passes"
    );
}

/// 2. First score 0.3, retry scores 0.8. Returns Accepted with attempt=2.
#[tokio::test]
async fn test_retry_improves_on_second_attempt() {
    let config = QualityRetryConfig {
        max_retries: 3,
        quality_threshold: 0.6,
    };
    let agent = make_agent("implementer", true, 0.6);
    let initial_output = "mediocre output";
    let initial_score = QualityScore {
        overall: 0.3,
        dimensions: HashMap::new(),
    };

    // run_agent will be called once; it returns score 0.8 on that call.
    let counter = Arc::new(AtomicU32::new(0));
    let run_agent = make_scoring_agent(vec![0.8], counter.clone());

    let result = retry_on_quality(&config, &agent, initial_output, &initial_score, run_agent)
        .await
        .expect("should not error");

    match result {
        QualityRetryResult::Accepted {
            output,
            score,
            attempt,
        } => {
            assert_eq!(attempt, 2, "should succeed on second attempt (1 retry)");
            assert!((score.overall - 0.8).abs() < f64::EPSILON);
            assert_eq!(output, "output attempt 1");
        }
        other => panic!("expected Accepted, got {:?}", variant_name(&other)),
    }

    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "run_agent should be called exactly once"
    );
}

/// 3. Critical agent, all attempts score 0.1. Returns Failed after 4 total
///    attempts (1 initial + 3 retries).
#[tokio::test]
async fn test_critical_agent_fails_after_max_retries() {
    let config = QualityRetryConfig {
        max_retries: 3,
        quality_threshold: 0.6,
    };
    let agent = make_agent("critical-validator", true, 0.6);
    let initial_output = "bad output";
    let initial_score = QualityScore {
        overall: 0.1,
        dimensions: HashMap::new(),
    };

    // All 3 retries return score 0.1
    let counter = Arc::new(AtomicU32::new(0));
    let run_agent = make_scoring_agent(vec![0.1, 0.1, 0.1], counter.clone());

    let result = retry_on_quality(&config, &agent, initial_output, &initial_score, run_agent)
        .await
        .expect("should not error");

    match result {
        QualityRetryResult::Failed {
            agent_key,
            final_score,
            attempts,
            reason,
        } => {
            assert_eq!(agent_key, "critical-validator");
            assert!((final_score.overall - 0.1).abs() < f64::EPSILON);
            assert_eq!(attempts, 4, "1 initial + 3 retries = 4 total attempts");
            assert!(
                !reason.is_empty(),
                "reason should describe why the agent failed"
            );
        }
        other => panic!("expected Failed, got {:?}", variant_name(&other)),
    }

    assert_eq!(
        counter.load(Ordering::SeqCst),
        3,
        "run_agent should be called exactly 3 times (max_retries)"
    );
}

/// 4. Non-critical agent, all attempts score 0.1. Returns Skipped with warning.
#[tokio::test]
async fn test_non_critical_agent_skipped_after_max_retries() {
    let config = QualityRetryConfig {
        max_retries: 3,
        quality_threshold: 0.6,
    };
    let agent = make_agent("optional-linter", false, 0.6);
    let initial_output = "bad output";
    let initial_score = QualityScore {
        overall: 0.1,
        dimensions: HashMap::new(),
    };

    let counter = Arc::new(AtomicU32::new(0));
    let run_agent = make_scoring_agent(vec![0.1, 0.1, 0.1], counter.clone());

    let result = retry_on_quality(&config, &agent, initial_output, &initial_score, run_agent)
        .await
        .expect("should not error");

    match result {
        QualityRetryResult::Skipped {
            agent_key,
            final_score,
            attempts,
            warning,
        } => {
            assert_eq!(agent_key, "optional-linter");
            assert!((final_score.overall - 0.1).abs() < f64::EPSILON);
            assert_eq!(attempts, 4, "1 initial + 3 retries = 4 total attempts");
            assert!(
                !warning.is_empty(),
                "warning should describe why the agent was skipped"
            );
        }
        other => panic!("expected Skipped, got {:?}", variant_name(&other)),
    }

    assert_eq!(
        counter.load(Ordering::SeqCst),
        3,
        "run_agent should be called exactly 3 times (max_retries)"
    );
}

/// 5. Initial output is empty (""), which should score 0.0 and trigger retry
///    with feedback containing "empty".
#[tokio::test]
async fn test_empty_output_triggers_retry() {
    let config = QualityRetryConfig {
        max_retries: 3,
        quality_threshold: 0.6,
    };
    let agent = make_agent("writer", true, 0.6);
    let initial_output = "";
    let initial_score = QualityScore {
        overall: 0.0,
        dimensions: HashMap::new(),
    };

    let counter = Arc::new(AtomicU32::new(0));
    let feedbacks = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let run_agent =
        make_scoring_agent_with_feedback_capture(vec![0.9], counter.clone(), feedbacks.clone());

    let result = retry_on_quality(&config, &agent, initial_output, &initial_score, run_agent)
        .await
        .expect("should not error");

    // Should have retried and then accepted on the retry.
    match result {
        QualityRetryResult::Accepted { attempt, .. } => {
            assert_eq!(attempt, 2, "should succeed on second attempt after retry");
        }
        other => panic!("expected Accepted, got {:?}", variant_name(&other)),
    }

    // Verify the feedback mentioned "empty".
    let captured = feedbacks.lock().unwrap();
    assert!(!captured.is_empty(), "should have captured feedback");
    let feedback_lower = captured[0].to_lowercase();
    assert!(
        feedback_lower.contains("empty"),
        "feedback should mention 'empty' output, got: {}",
        captured[0]
    );
}

/// 6. Score 0.0 should retry with comprehensive feedback.
#[tokio::test]
async fn test_zero_score_triggers_retry() {
    let config = QualityRetryConfig {
        max_retries: 3,
        quality_threshold: 0.6,
    };
    let agent = make_agent("scorer-zero", true, 0.6);
    let initial_output = "some output that scores zero";
    let initial_score = QualityScore {
        overall: 0.0,
        dimensions: HashMap::new(),
    };

    let counter = Arc::new(AtomicU32::new(0));
    let feedbacks = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let run_agent =
        make_scoring_agent_with_feedback_capture(vec![0.7], counter.clone(), feedbacks.clone());

    let result = retry_on_quality(&config, &agent, initial_output, &initial_score, run_agent)
        .await
        .expect("should not error");

    match result {
        QualityRetryResult::Accepted { attempt, .. } => {
            assert_eq!(attempt, 2);
        }
        other => panic!("expected Accepted, got {:?}", variant_name(&other)),
    }

    let captured = feedbacks.lock().unwrap();
    assert!(
        !captured.is_empty(),
        "should have generated feedback for zero score"
    );
    // Feedback for a zero score should be substantive (not just a few characters).
    assert!(
        captured[0].len() > 10,
        "feedback should be comprehensive for zero score, got: {}",
        captured[0]
    );
}

/// 7. QualityScore with dimensions {"completeness": 0.2, "accuracy": 0.9},
///    feedback should mention low "completeness".
#[tokio::test]
async fn test_feedback_contains_dimension_scores() {
    let mut dimensions = HashMap::new();
    dimensions.insert("completeness".to_string(), 0.2);
    dimensions.insert("accuracy".to_string(), 0.9);

    let score = QualityScore {
        overall: 0.4,
        dimensions,
    };

    let output = "partial output missing several sections";
    let feedback = build_quality_feedback(&score, output);

    let feedback_lower = feedback.to_lowercase();
    assert!(
        feedback_lower.contains("completeness"),
        "feedback should mention the low-scoring 'completeness' dimension, got: {}",
        feedback
    );
    // Accuracy is high, so it may or may not appear, but completeness MUST.
}

/// 8. Exactly max_retries+1 total attempts are made.
#[tokio::test]
async fn test_max_retries_honored() {
    let max_retries = 3u32;
    let config = QualityRetryConfig {
        max_retries,
        quality_threshold: 0.6,
    };
    let agent = make_agent("retry-counter", true, 0.6);
    let initial_output = "low quality";
    let initial_score = QualityScore {
        overall: 0.1,
        dimensions: HashMap::new(),
    };

    // All retries return low scores.
    let counter = Arc::new(AtomicU32::new(0));
    let run_agent = make_scoring_agent(vec![0.1, 0.1, 0.1, 0.1, 0.1], counter.clone());

    let result = retry_on_quality(&config, &agent, initial_output, &initial_score, run_agent)
        .await
        .expect("should not error");

    // Should have exhausted retries.
    match result {
        QualityRetryResult::Failed { attempts, .. } => {
            assert_eq!(
                attempts,
                max_retries + 1,
                "total attempts should be exactly max_retries+1 = {}",
                max_retries + 1
            );
        }
        other => panic!("expected Failed, got {:?}", variant_name(&other)),
    }

    assert_eq!(
        counter.load(Ordering::SeqCst),
        max_retries,
        "run_agent should be called exactly max_retries={} times",
        max_retries
    );
}

/// 9. Verify QualityRetryConfig default: max_retries=3, quality_threshold=0.6.
#[tokio::test]
async fn test_default_quality_retry_config() {
    let config = QualityRetryConfig::default();

    assert_eq!(config.max_retries, 3, "default max_retries should be 3");
    assert!(
        (config.quality_threshold - 0.6).abs() < f64::EPSILON,
        "default quality_threshold should be 0.6"
    );
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

/// Returns the variant name as a string for nicer error messages.
fn variant_name(result: &QualityRetryResult) -> &'static str {
    match result {
        QualityRetryResult::Accepted { .. } => "Accepted",
        QualityRetryResult::Failed { .. } => "Failed",
        QualityRetryResult::Skipped { .. } => "Skipped",
    }
}
