//! Tests for SONA Engine (TASK-PIPE-F01).
//!
//! Validates: trajectory creation, feedback/weight update, CRC32
//! binary persistence, drift detection, checkpoint/rollback,
//! step capture, weight math accuracy.

use archon_pipeline::learning::sona::{
    DriftStatus, FeedbackInput, SonaConfig, SonaEngine, StepCaptureService, calculate_gradient, calculate_reward,
    calculate_weight_update, cosine_similarity, crc32_checksum, update_fisher_information,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn default_config() -> SonaConfig {
    SonaConfig::default()
}

// ---------------------------------------------------------------------------
// Weight math tests
// ---------------------------------------------------------------------------

mod math_tests {
    use super::*;

    #[test]
    fn reward_formula_matches_typescript() {
        // reward = quality * l_score * success_rate
        let r = calculate_reward(0.8, 0.9, 1.0);
        assert!((r - 0.72).abs() < 1e-6, "0.8 * 0.9 * 1.0 = 0.72, got {}", r);
    }

    #[test]
    fn gradient_formula_matches_typescript() {
        // gradient = (reward - 0.5) * similarity
        let g = calculate_gradient(0.8, 0.6);
        assert!(
            (g - 0.18).abs() < 1e-6,
            "(0.8 - 0.5) * 0.6 = 0.18, got {}",
            g
        );
    }

    #[test]
    fn weight_update_with_regularization() {
        // weight_change = learning_rate * gradient / (1 + regularization * importance)
        let w = calculate_weight_update(0.18, 0.01, 0.1, 0.5);
        let expected = 0.01 * 0.18 / (1.0 + 0.1 * 0.5);
        assert!(
            (w - expected).abs() < 1e-8,
            "expected {}, got {}",
            expected,
            w
        );
    }

    #[test]
    fn fisher_information_update() {
        // fisher = decay * old + (1 - decay) * gradient^2
        let f = update_fisher_information(1.0, 0.5, 0.9);
        let expected = 0.9 * 1.0 + 0.1 * 0.25; // 0.925
        assert!(
            (f - expected).abs() < 1e-8,
            "expected {}, got {}",
            expected,
            f
        );
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-6, "zero vector should return 0.0");
    }

    #[test]
    fn weight_clamped_to_range() {
        // Very large gradient should still clamp weight to [-1, 1]
        let w = calculate_weight_update(100.0, 1.0, 0.0, 0.0);
        assert!(w >= -1.0 && w <= 1.0, "weight should be clamped, got {}", w);
    }
}

// ---------------------------------------------------------------------------
// CRC32 tests
// ---------------------------------------------------------------------------

mod crc32_tests {
    use super::*;

    #[test]
    fn crc32_known_value() {
        // CRC32 of empty bytes with polynomial 0xEDB88320
        let crc = crc32_checksum(&[]);
        assert_eq!(crc, 0x00000000, "empty input CRC32 should be 0");
    }

    #[test]
    fn crc32_deterministic() {
        let data = b"hello world";
        let c1 = crc32_checksum(data);
        let c2 = crc32_checksum(data);
        assert_eq!(c1, c2, "CRC32 should be deterministic");
    }

    #[test]
    fn crc32_different_inputs_differ() {
        let c1 = crc32_checksum(b"hello");
        let c2 = crc32_checksum(b"world");
        assert_ne!(c1, c2, "different inputs should produce different CRC32");
    }
}

// ---------------------------------------------------------------------------
// Trajectory and feedback tests
// ---------------------------------------------------------------------------

mod trajectory_tests {
    use super::*;

    #[test]
    fn create_trajectory() {
        let mut engine = SonaEngine::new(default_config());
        let traj = engine.create_trajectory("route-a", "code-generator", "session-1");
        assert!(!traj.trajectory_id.is_empty());
        assert_eq!(traj.route, "route-a");
        assert_eq!(traj.agent_key, "code-generator");
    }

    #[test]
    fn provide_feedback_updates_trajectory() {
        let mut engine = SonaEngine::new(default_config());
        let traj = engine.create_trajectory("route-a", "code-generator", "session-1");
        let tid = traj.trajectory_id.clone();

        let feedback = FeedbackInput {
            trajectory_id: tid.clone(),
            quality: 0.85,
            l_score: 0.9,
            success_rate: 1.0,
        };
        let result = engine.provide_feedback(&feedback);
        assert!(
            result.is_ok(),
            "feedback should succeed: {:?}",
            result.err()
        );

        let updated = engine.get_trajectory(&tid);
        assert!(updated.is_some());
        let t = updated.unwrap();
        assert!(t.quality > 0.0);
    }

    #[test]
    fn feedback_on_nonexistent_trajectory_errors() {
        let mut engine = SonaEngine::new(default_config());
        let feedback = FeedbackInput {
            trajectory_id: "nonexistent".into(),
            quality: 0.5,
            l_score: 0.5,
            success_rate: 0.5,
        };
        let result = engine.provide_feedback(&feedback);
        assert!(result.is_err());
    }

    #[test]
    fn weight_updated_after_feedback() {
        let mut engine = SonaEngine::new(default_config());
        let traj = engine.create_trajectory("route-a", "code-generator", "session-1");

        let feedback = FeedbackInput {
            trajectory_id: traj.trajectory_id.clone(),
            quality: 0.9,
            l_score: 0.95,
            success_rate: 1.0,
        };
        engine.provide_feedback(&feedback).unwrap();

        let weight = engine.get_weight("route-a", "default");
        // Weight should have moved from 0.0 toward positive (good feedback)
        assert!(weight != 0.0, "weight should have been updated");
    }
}

// ---------------------------------------------------------------------------
// Drift detection tests
// ---------------------------------------------------------------------------

mod drift_tests {
    use super::*;

    #[test]
    fn drift_normal_for_small_changes() {
        let engine = SonaEngine::new(default_config());
        let old_weights = vec![0.1, 0.2, 0.3];
        let new_weights = vec![0.11, 0.21, 0.31];
        let report = engine.check_drift(&old_weights, &new_weights);
        assert_eq!(report.status, DriftStatus::Normal);
    }

    #[test]
    fn drift_alert_for_moderate_changes() {
        let engine = SonaEngine::new(default_config());
        let old_weights = vec![1.0, 0.0, 0.0];
        let new_weights = vec![0.5, 0.5, 0.5]; // significant divergence
        let report = engine.check_drift(&old_weights, &new_weights);
        assert!(
            matches!(report.status, DriftStatus::Alert | DriftStatus::Reject),
            "moderate change should trigger at least Alert"
        );
    }
}

// ---------------------------------------------------------------------------
// Checkpoint / rollback tests
// ---------------------------------------------------------------------------

mod checkpoint_tests {
    use super::*;

    #[test]
    fn checkpoint_and_rollback() {
        let mut engine = SonaEngine::new(default_config());
        let traj = engine.create_trajectory("route-a", "agent-1", "session-1");

        // Checkpoint before feedback
        engine.save_checkpoint();

        // Provide feedback that changes weights
        engine
            .provide_feedback(&FeedbackInput {
                trajectory_id: traj.trajectory_id.clone(),
                quality: 0.9,
                l_score: 0.95,
                success_rate: 1.0,
            })
            .unwrap();

        let weight_after = engine.get_weight("route-a", "default");
        assert!(weight_after != 0.0);

        // Rollback
        let rolled_back = engine.rollback();
        assert!(rolled_back, "rollback should succeed");

        let weight_restored = engine.get_weight("route-a", "default");
        assert!(
            (weight_restored - 0.0).abs() < 1e-6,
            "weight should be restored to 0.0"
        );
    }

    #[test]
    fn rollback_with_no_checkpoints_returns_false() {
        let mut engine = SonaEngine::new(default_config());
        assert!(
            !engine.rollback(),
            "rollback with no checkpoints should return false"
        );
    }

    #[test]
    fn max_checkpoints_respected() {
        let mut engine = SonaEngine::new(SonaConfig {
            max_checkpoints: 3,
            ..default_config()
        });

        for _ in 0..5 {
            engine.save_checkpoint();
        }

        assert!(engine.checkpoint_count() <= 3, "should not exceed max");
    }
}

// ---------------------------------------------------------------------------
// Step capture tests
// ---------------------------------------------------------------------------

mod step_capture_tests {
    use super::*;

    #[test]
    fn capture_steps_for_trajectory() {
        let mut service = StepCaptureService::new();
        service.begin_capture("traj-1");

        service.capture_step("traj-1", "action-1", "observation-1", 0.5);
        service.capture_step("traj-1", "action-2", "observation-2", 0.8);

        let steps = service.end_capture("traj-1");
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].action, "action-1");
        assert_eq!(steps[1].step_index, 1);
    }

    #[test]
    fn end_capture_clears_buffer() {
        let mut service = StepCaptureService::new();
        service.begin_capture("traj-1");
        service.capture_step("traj-1", "act", "obs", 0.5);
        let _ = service.end_capture("traj-1");

        let steps = service.end_capture("traj-1");
        assert!(
            steps.is_empty(),
            "buffer should be cleared after end_capture"
        );
    }

    #[test]
    fn large_observation_truncated() {
        let mut service = StepCaptureService::new();
        service.begin_capture("traj-1");

        let large_obs = "x".repeat(20_000);
        service.capture_step("traj-1", "act", &large_obs, 0.5);

        let steps = service.end_capture("traj-1");
        assert!(
            steps[0].observation.len() <= 10_000,
            "observation should be truncated"
        );
    }
}
