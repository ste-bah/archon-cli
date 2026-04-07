//! Tests for PatternMatcher + PatternStore + confidence-scorer (TASK-PIPE-F02).
//!
//! Validates: confidence scoring, sigmoid calibration, ranking, pattern CRUD,
//! duplicate detection, EMA update, pruning, task-type filtering.

use archon_pipeline::learning::confidence::{
    calculate_confidence, calibrate_confidence, rank_patterns, filter_patterns,
    batch_calculate_confidence,
};
use archon_pipeline::learning::patterns::{
    PatternStore, Pattern, TaskType, CreatePatternParams, PruneParams,
};
use archon_pipeline::learning::sona::cosine_similarity;

// ---------------------------------------------------------------------------
// Confidence scorer tests
// ---------------------------------------------------------------------------

mod confidence_tests {
    use super::*;

    #[test]
    fn confidence_formula_basic() {
        // confidence = similarity * success_rate * sona_weight
        let c = calculate_confidence(0.9, 0.8, 0.7);
        let expected = 0.9 * 0.8 * 0.7;
        assert!((c - expected).abs() < 1e-6, "expected {}, got {}", expected, c);
    }

    #[test]
    fn confidence_clamps_inputs() {
        // Inputs outside [0,1] should be clamped
        let c = calculate_confidence(1.5, -0.2, 0.8);
        assert!(c >= 0.0 && c <= 1.0, "confidence should be in [0,1], got {}", c);
    }

    #[test]
    fn sigmoid_calibration_at_midpoint() {
        // sigmoid(0.5) with steepness=10, centered at 0.5 should be ~0.5
        let cal = calibrate_confidence(0.5, 10.0);
        assert!((cal - 0.5).abs() < 0.01, "sigmoid at 0.5 should be ~0.5, got {}", cal);
    }

    #[test]
    fn sigmoid_calibration_high_value() {
        // sigmoid(0.9) should be close to 1.0
        let cal = calibrate_confidence(0.9, 10.0);
        assert!(cal > 0.95, "sigmoid(0.9) should be > 0.95, got {}", cal);
    }

    #[test]
    fn sigmoid_calibration_low_value() {
        // sigmoid(0.1) should be close to 0.0
        let cal = calibrate_confidence(0.1, 10.0);
        assert!(cal < 0.05, "sigmoid(0.1) should be < 0.05, got {}", cal);
    }

    #[test]
    fn ranking_by_confidence_desc() {
        let patterns = vec![
            ("p1", 0.3f64),
            ("p2", 0.9),
            ("p3", 0.6),
        ];
        let ranked = rank_patterns(&patterns);
        assert_eq!(ranked[0].0, "p2");
        assert_eq!(ranked[1].0, "p3");
        assert_eq!(ranked[2].0, "p1");
    }

    #[test]
    fn filter_by_min_confidence() {
        let patterns = vec![
            ("p1", 0.3f64),
            ("p2", 0.9),
            ("p3", 0.6),
        ];
        let filtered = filter_patterns(&patterns, 0.5);
        assert_eq!(filtered.len(), 2);
        assert!(filtered.iter().all(|p| p.1 >= 0.5));
    }

    #[test]
    fn batch_confidence_calculation() {
        let inputs = vec![
            (0.9, 0.8, 0.7),
            (0.5, 0.5, 0.5),
        ];
        let results = batch_calculate_confidence(&inputs);
        assert_eq!(results.len(), 2);
        assert!((results[0] - 0.504).abs() < 0.01, "first: {}", results[0]);
    }
}

// ---------------------------------------------------------------------------
// PatternStore tests
// ---------------------------------------------------------------------------

mod pattern_store_tests {
    use super::*;

    fn sample_embedding() -> Vec<f64> {
        vec![0.1, 0.2, 0.3, 0.4, 0.5]
    }

    #[test]
    fn create_and_get_pattern() {
        let mut store = PatternStore::new();
        let params = CreatePatternParams {
            task_type: TaskType::Coding,
            template: "Use TDD approach".into(),
            embedding: sample_embedding(),
            initial_success_rate: 0.5,
        };
        let pattern = store.create_pattern(params).unwrap();
        assert!(!pattern.id.is_empty());
        assert_eq!(pattern.task_type, TaskType::Coding);

        let retrieved = store.get_pattern(&pattern.id);
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().template, "Use TDD approach");
    }

    #[test]
    fn create_rejects_low_success_rate() {
        let mut store = PatternStore::new();
        let params = CreatePatternParams {
            task_type: TaskType::Coding,
            template: "Bad pattern".into(),
            embedding: sample_embedding(),
            initial_success_rate: 0.05, // below 0.1 floor
        };
        let result = store.create_pattern(params);
        assert!(result.is_err(), "should reject success_rate < 0.1");
    }

    #[test]
    fn get_patterns_by_task_type() {
        let mut store = PatternStore::new();

        store.create_pattern(CreatePatternParams {
            task_type: TaskType::Coding,
            template: "Coding pattern".into(),
            embedding: vec![1.0, 0.0, 0.0],
            initial_success_rate: 0.5,
        }).unwrap();

        store.create_pattern(CreatePatternParams {
            task_type: TaskType::Research,
            template: "Research pattern".into(),
            embedding: vec![0.0, 1.0, 0.0],
            initial_success_rate: 0.5,
        }).unwrap();

        let coding = store.get_patterns_by_task_type(&TaskType::Coding);
        assert_eq!(coding.len(), 1);
        assert_eq!(coding[0].template, "Coding pattern");
    }

    #[test]
    fn ema_success_rate_update() {
        let mut store = PatternStore::new();
        let pattern = store.create_pattern(CreatePatternParams {
            task_type: TaskType::Coding,
            template: "Pattern".into(),
            embedding: sample_embedding(),
            initial_success_rate: 0.5,
        }).unwrap();

        // EMA: new = 0.2 * value + 0.8 * old
        store.update_success_rate(&pattern.id, 1.0); // new = 0.2*1.0 + 0.8*0.5 = 0.6
        let updated = store.get_pattern(&pattern.id).unwrap();
        let expected = 0.2 * 1.0 + 0.8 * 0.5;
        assert!((updated.success_rate - expected).abs() < 1e-6, "expected {}, got {}", expected, updated.success_rate);
    }

    #[test]
    fn sona_weight_update() {
        let mut store = PatternStore::new();
        let pattern = store.create_pattern(CreatePatternParams {
            task_type: TaskType::Coding,
            template: "Pattern".into(),
            embedding: sample_embedding(),
            initial_success_rate: 0.5,
        }).unwrap();

        store.update_sona_weight(&pattern.id, 0.75);
        let updated = store.get_pattern(&pattern.id).unwrap();
        assert!((updated.sona_weight - 0.75).abs() < 1e-6);
    }

    #[test]
    fn duplicate_detection() {
        let mut store = PatternStore::new();

        store.create_pattern(CreatePatternParams {
            task_type: TaskType::Coding,
            template: "Original".into(),
            embedding: vec![1.0, 0.0, 0.0, 0.0, 0.0],
            initial_success_rate: 0.5,
        }).unwrap();

        // Nearly identical embedding (cosine sim > 0.95)
        let result = store.create_pattern(CreatePatternParams {
            task_type: TaskType::Coding,
            template: "Duplicate".into(),
            embedding: vec![0.99, 0.01, 0.0, 0.0, 0.0],
            initial_success_rate: 0.5,
        });
        assert!(result.is_err(), "should reject duplicate (cosine > 0.95)");
    }

    #[test]
    fn prune_low_quality_patterns() {
        let mut store = PatternStore::new();

        // Low quality, high usage (should be pruned)
        let p1 = store.create_pattern(CreatePatternParams {
            task_type: TaskType::Coding,
            template: "Bad pattern".into(),
            embedding: vec![1.0, 0.0, 0.0],
            initial_success_rate: 0.15,
        }).unwrap();
        // Bump usage count to >= 5
        for _ in 0..5 {
            store.increment_usage(&p1.id);
        }

        // Good quality (should not be pruned)
        store.create_pattern(CreatePatternParams {
            task_type: TaskType::Coding,
            template: "Good pattern".into(),
            embedding: vec![0.0, 1.0, 0.0],
            initial_success_rate: 0.8,
        }).unwrap();

        let result = store.prune(PruneParams {
            min_success_rate: 0.20,
            min_usage_count: 5,
        });
        assert_eq!(result.pruned_count, 1);
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn delete_pattern() {
        let mut store = PatternStore::new();
        let p = store.create_pattern(CreatePatternParams {
            task_type: TaskType::Coding,
            template: "To delete".into(),
            embedding: sample_embedding(),
            initial_success_rate: 0.5,
        }).unwrap();

        assert!(store.delete_pattern(&p.id));
        assert!(store.get_pattern(&p.id).is_none());
    }

    #[test]
    fn stats_reporting() {
        let mut store = PatternStore::new();
        store.create_pattern(CreatePatternParams {
            task_type: TaskType::Coding,
            template: "P1".into(),
            embedding: vec![1.0, 0.0],
            initial_success_rate: 0.5,
        }).unwrap();
        store.create_pattern(CreatePatternParams {
            task_type: TaskType::Research,
            template: "P2".into(),
            embedding: vec![0.0, 1.0],
            initial_success_rate: 0.7,
        }).unwrap();

        let stats = store.stats();
        assert_eq!(stats.total_patterns, 2);
        assert_eq!(*stats.by_task_type.get("Coding").unwrap_or(&0), 1);
        assert_eq!(*stats.by_task_type.get("Research").unwrap_or(&0), 1);
    }
}
