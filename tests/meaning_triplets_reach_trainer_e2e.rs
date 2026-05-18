use std::sync::Arc;
use std::time::Duration;

use archon_meaning::HydratedTriplet;
use archon_pipeline::learning::gnn::auto_trainer::{AutoTrainer, AutoTrainerConfig};
use archon_pipeline::learning::gnn::cache::CacheConfig;
use archon_pipeline::learning::gnn::loss::TrajectoryWithFeedback;
use archon_pipeline::learning::gnn::trainer::TrainingConfig;
use archon_pipeline::learning::gnn::triplets_loss::TripletBatch;
use archon_pipeline::learning::gnn::weights::WeightStore;
use archon_pipeline::learning::gnn::{GnnConfig, GnnEnhancer};

#[test]
fn meaning_triplets_reach_trainer_e2e() {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let enhancer = Arc::new(GnnEnhancer::with_in_memory_weights(
            GnnConfig {
                input_dim: 4,
                output_dim: 4,
                num_layers: 3,
                attention_heads: 1,
                dropout: 0.0,
                max_nodes: 4,
                use_residual: true,
                use_layer_norm: true,
                activation: archon_pipeline::learning::gnn::math::ActivationType::Relu,
            },
            CacheConfig::default(),
            42,
        ));
        let weights = Arc::new(WeightStore::with_in_memory());
        let trainer = Arc::new(AutoTrainer::new(AutoTrainerConfig {
            enabled: true,
            first_run_threshold: 1,
            tick_interval_ms: 10,
            min_throttle_ms: 0,
            max_runtime_ms: 30_000,
            ..AutoTrainerConfig::default()
        }));
        let sample_provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync> =
            Arc::new(Vec::new);
        let triplet_provider: Arc<dyn Fn() -> TripletBatch + Send + Sync> =
            Arc::new(|| TripletBatch {
                triplets: vec![
                    hydrated(
                        "t1",
                        [0.0, 0.0, 0.0, 0.0],
                        [2.0, 0.0, 0.0, 0.0],
                        [0.01, 0.0, 0.0, 0.0],
                    ),
                    hydrated(
                        "t2",
                        [0.0, 0.1, 0.0, 0.0],
                        [2.0, 0.1, 0.0, 0.0],
                        [0.01, 0.1, 0.0, 0.0],
                    ),
                    hydrated(
                        "t3",
                        [0.0, 0.0, 0.1, 0.0],
                        [2.0, 0.0, 0.1, 0.0],
                        [0.01, 0.0, 0.1, 0.0],
                    ),
                ],
            });

        trainer.spawn_with_triplet_provider(
            enhancer,
            weights,
            TrainingConfig {
                max_epochs: 1,
                batch_size: 4,
                max_runtime_ms: 30_000,
                ..TrainingConfig::default()
            },
            sample_provider,
            triplet_provider,
        );
        trainer.record_memories(1);
        for _ in 0..100 {
            let status = trainer.status();
            if status.training_count > 0 && !status.training_in_progress {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let status = trainer.status();
        assert!(
            status.training_count > 0,
            "auto-trainer did not complete a training run"
        );
        let outcome = status.last_outcome.expect("last training outcome");
        assert!(
            outcome
                .epoch_metrics
                .iter()
                .any(|epoch| epoch.loss_triplet > 0.0),
            "expected a non-zero triplet loss epoch, got {:?}",
            outcome.epoch_metrics
        );
        trainer.shutdown();
    });
}

fn hydrated(id: &str, anchor: [f32; 4], positive: [f32; 4], negative: [f32; 4]) -> HydratedTriplet {
    HydratedTriplet {
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
