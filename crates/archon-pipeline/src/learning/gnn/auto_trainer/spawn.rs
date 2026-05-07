use std::sync::Arc;

use tracing::info;

use super::super::GnnEnhancer;
use super::super::loss::TrajectoryWithFeedback;
use super::super::trainer::TrainingConfig;
use super::super::triplets_loss::TripletBatch;
use super::super::weights::WeightStore;
use super::types::AutoTrainer;

impl AutoTrainer {
    /// Launch the background training loop.
    ///
    /// If `config.enabled` is false, logs and returns without spawning.
    ///
    /// `sample_provider` is called on the async runtime thread before
    /// `spawn_blocking` offloads the sync training — it should return
    /// quickly (just query the DB, no heavy computation).
    pub fn spawn(
        &self,
        enhancer: Arc<GnnEnhancer>,
        weight_store: Arc<WeightStore>,
        train_cfg: TrainingConfig,
        sample_provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync>,
    ) {
        self.spawn_with_triplet_provider(
            enhancer,
            weight_store,
            train_cfg,
            sample_provider,
            Arc::new(TripletBatch::default),
        );
    }

    /// Launch the background training loop with hydrated meaning triplets.
    pub fn spawn_with_triplet_provider(
        &self,
        enhancer: Arc<GnnEnhancer>,
        weight_store: Arc<WeightStore>,
        train_cfg: TrainingConfig,
        sample_provider: Arc<dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync>,
        triplet_provider: Arc<dyn Fn() -> TripletBatch + Send + Sync>,
    ) {
        if !self.config.enabled {
            info!("AutoTrainer: disabled, background task not started");
            return;
        }

        let state = Arc::clone(&self.state);
        let config = self.config.clone();
        let cancel = self.cancel.clone();
        let training_cancel = Arc::clone(&self.training_cancel);

        let handle = archon_observability::spawn_named("gnn-auto-trainer", async move {
            Self::run_loop(
                config,
                state,
                enhancer,
                weight_store,
                train_cfg,
                sample_provider,
                triplet_provider,
                cancel,
                training_cancel,
            )
            .await;
        });

        self.handle.lock().unwrap().replace(handle);
        info!("AutoTrainer: background task spawned");
    }
}
