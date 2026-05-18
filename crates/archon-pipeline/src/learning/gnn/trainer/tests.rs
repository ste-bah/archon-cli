use super::*;
use crate::learning::gnn::cache::CacheConfig;
use crate::learning::gnn::loss::TrajectoryWithFeedback;
use crate::learning::gnn::triplets_loss::TripletBatch;
use crate::learning::gnn::{GnnConfig, GnnEnhancer};

fn make_sample(id: &str, embedding: Vec<f32>, quality: f32) -> TrajectoryWithFeedback {
    TrajectoryWithFeedback {
        trajectory_id: id.to_string(),
        embedding,
        quality,
    }
}

fn test_enhancer() -> GnnEnhancer {
    GnnEnhancer::with_in_memory_weights(GnnConfig::default(), CacheConfig::default(), 42)
}

#[test]
fn test_trainer_requires_enough_samples() {
    let mut trainer = GnnTrainer::new(TrainingConfig::default(), None);
    let enhancer = test_enhancer();
    let samples = vec![
        make_sample("a", vec![0.1; 4], 0.9),
        make_sample("b", vec![0.2; 4], 0.1),
    ];
    let outcome = trainer.train(&enhancer, &samples, None);
    assert_eq!(outcome.epochs_completed, 0);
    assert_eq!(outcome.data_sources.sona_trajectories, 2);
    assert_eq!(outcome.data_sources.meaning_triplets, 0);
    assert_eq!(
        outcome.data_sources.no_data_reason(),
        Some("insufficient_sona_triplets_and_no_meaning_triplets")
    );
}

#[test]
fn test_trainer_runs_with_valid_samples() {
    let mut trainer = GnnTrainer::new(
        TrainingConfig {
            max_epochs: 1,
            batch_size: 4,
            max_triplets_per_run: 16,
            max_runtime_ms: 30_000,
            ..TrainingConfig::default()
        },
        None,
    );
    let enhancer = test_enhancer();
    let samples: Vec<TrajectoryWithFeedback> = (0..10)
        .map(|i| {
            let q = if i % 2 == 0 { 0.9 } else { 0.1 };
            make_sample(&format!("t{}", i), vec![i as f32 * 0.1; 4], q)
        })
        .collect();

    let outcome = trainer.train(&enhancer, &samples, None);
    assert!(outcome.epochs_completed >= 1);
    assert!(outcome.batches_processed > 0);
    assert!(outcome.sona_samples_processed > 0);
    assert_eq!(outcome.meaning_triplets_processed, 0);
}

#[test]
fn trainer_handles_empty_triplet_batch() {
    let mut trainer = GnnTrainer::new(
        TrainingConfig {
            max_epochs: 1,
            batch_size: 4,
            max_triplets_per_run: 16,
            max_runtime_ms: 30_000,
            ..TrainingConfig::default()
        },
        None,
    );
    let enhancer = test_enhancer();
    let samples: Vec<TrajectoryWithFeedback> = (0..10)
        .map(|i| {
            let q = if i % 2 == 0 { 0.9 } else { 0.1 };
            make_sample(&format!("t{}", i), vec![i as f32 * 0.1; 4], q)
        })
        .collect();

    let outcome = trainer.train_with_triplets(&enhancer, &samples, &TripletBatch::default(), None);

    assert!(outcome.epochs_completed >= 1);
    assert!(
        outcome
            .epoch_metrics
            .iter()
            .all(|epoch| epoch.loss_triplet == 0.0)
    );
    assert_eq!(outcome.data_sources.meaning_triplets, 0);
}

#[test]
fn trainer_reports_separate_sona_and_meaning_counts() {
    let mut trainer = GnnTrainer::new(
        TrainingConfig {
            max_epochs: 1,
            batch_size: 4,
            max_triplets_per_run: 16,
            max_runtime_ms: 30_000,
            ..TrainingConfig::default()
        },
        None,
    );
    let enhancer = test_enhancer();
    let samples: Vec<TrajectoryWithFeedback> = (0..10)
        .map(|i| {
            let q = if i % 2 == 0 { 0.9 } else { 0.1 };
            make_sample(&format!("t{}", i), vec![i as f32 * 0.1; 4], q)
        })
        .collect();
    let triplet_batch = TripletBatch {
        triplets: vec![
            hydrated(
                "m1",
                [0.0, 0.0, 0.0, 0.0],
                [0.1, 0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0, 0.0],
            ),
            hydrated(
                "m2",
                [0.0, 0.1, 0.0, 0.0],
                [0.1, 0.1, 0.0, 0.0],
                [1.0, 0.1, 0.0, 0.0],
            ),
        ],
    };

    let outcome = trainer.train_with_triplets(&enhancer, &samples, &triplet_batch, None);

    assert_eq!(outcome.data_sources.sona_trajectories, 10);
    assert!(outcome.data_sources.sona_triplets >= 2);
    assert_eq!(outcome.data_sources.meaning_triplets, 2);
    assert!(outcome.sona_samples_processed > 0);
    assert_eq!(outcome.meaning_triplets_processed, 2);
    assert!(outcome.data_sources.no_data_reason().is_none());
}

#[test]
fn trainer_reports_no_data_when_sources_empty() {
    let mut trainer = GnnTrainer::new(TrainingConfig::default(), None);
    let enhancer = test_enhancer();

    let outcome = trainer.train_with_triplets(&enhancer, &[], &TripletBatch::default(), None);

    assert_eq!(outcome.epochs_completed, 0);
    assert_eq!(outcome.data_sources.sona_trajectories, 0);
    assert_eq!(outcome.data_sources.sona_triplets, 0);
    assert_eq!(outcome.data_sources.meaning_triplets, 0);
    assert_eq!(
        outcome.data_sources.no_data_reason(),
        Some("no_sona_trajectories_or_meaning_triplets")
    );
}

#[test]
fn test_trainer_cancellation() {
    let mut trainer = GnnTrainer::new(
        TrainingConfig {
            max_epochs: 10,
            batch_size: 2,
            max_triplets_per_run: 16,
            max_runtime_ms: 30_000,
            ..TrainingConfig::default()
        },
        None,
    );
    let enhancer = test_enhancer();
    let samples: Vec<TrajectoryWithFeedback> = (0..10)
        .map(|i| {
            let q = if i % 2 == 0 { 0.9 } else { 0.1 };
            make_sample(&format!("t{}", i), vec![i as f32 * 0.1; 4], q)
        })
        .collect();

    let cancel = std::sync::atomic::AtomicBool::new(true);
    let outcome = trainer.train(&enhancer, &samples, Some(&cancel));
    assert!(outcome.cancelled);
}

#[test]
fn test_training_config_defaults() {
    let cfg = TrainingConfig::default();
    assert!((cfg.learning_rate - 0.001).abs() < 1e-6);
    assert_eq!(cfg.max_epochs, 10);
    assert_eq!(cfg.batch_size, 32);
}

#[test]
fn test_training_outcome_fields() {
    let outcome = TrainingOutcome {
        epochs_completed: 5,
        batches_processed: 20,
        samples_processed: 640,
        sona_samples_processed: 512,
        meaning_triplets_processed: 128,
        data_sources: TrainingDataSources {
            sona_trajectories: 100,
            sona_triplets: 80,
            meaning_triplets: 16,
        },
        initial_loss: 0.5,
        final_loss: 0.3,
        best_loss: 0.25,
        validation_loss: Some(0.28),
        stopped_early: false,
        timed_out: false,
        cancelled: false,
        best_epoch: 4,
        best_val_loss: 0.25,
        best_train_loss: 0.26,
        epoch_metrics: vec![],
    };
    assert!(outcome.final_loss < outcome.initial_loss);
    assert!(!outcome.cancelled);
}

fn hydrated(
    id: &str,
    anchor: [f32; 4],
    positive: [f32; 4],
    negative: [f32; 4],
) -> archon_meaning::HydratedTriplet {
    archon_meaning::HydratedTriplet {
        triplet_id: id.into(),
        workspace_id: "workspace".into(),
        anchor: anchor.to_vec(),
        positive: positive.to_vec(),
        negative: negative.to_vec(),
        embedding_sources: vec![
            archon_meaning::STORED_EMBEDDING_FEATURE_SPACE.into(),
            archon_meaning::STORED_EMBEDDING_FEATURE_SPACE.into(),
            archon_meaning::STORED_EMBEDDING_FEATURE_SPACE.into(),
        ],
    }
}
