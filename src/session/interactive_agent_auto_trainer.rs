use std::sync::Arc;

use archon_memory::MemoryTrait;

pub(super) fn build_auto_trainer(
    config: &archon_core::config::ArchonConfig,
    learning_cozo_db: &Option<Arc<cozo::DbInstance>>,
    memory: &dyn MemoryTrait,
) -> Option<Arc<archon_pipeline::learning::gnn::auto_trainer::AutoTrainer>> {
    let at_cfg = &config.learning.gnn.auto_trainer;
    if !at_cfg.enabled || !config.learning.gnn.enabled {
        tracing::info!(
            at_enabled = at_cfg.enabled,
            gnn_enabled = config.learning.gnn.enabled,
            "GNN auto-trainer disabled by config"
        );
        return None;
    }
    let Some(db) = learning_cozo_db.as_ref() else {
        tracing::warn!(
            "GNN auto-trainer enabled in config but learning CozoDB unavailable; not spawning"
        );
        return None;
    };

    let gnn_cfg = &config.learning.gnn;
    let train_cfg = &gnn_cfg.training;
    let seed = super::super::gnn_auto_trainer_seed::from_memory_graph(memory);
    let params = archon_pipeline::learning::gnn::auto_trainer_runtime::AutoTrainerBuildParams {
        at_config: archon_pipeline::learning::gnn::auto_trainer::AutoTrainerConfig {
            enabled: at_cfg.enabled,
            min_throttle_ms: at_cfg.min_throttle_ms,
            trigger_new_memories: at_cfg.trigger_new_memories,
            trigger_elapsed_ms: at_cfg.trigger_elapsed_ms,
            trigger_corrections: at_cfg.trigger_corrections,
            first_run_threshold: at_cfg.first_run_threshold,
            max_runtime_ms: at_cfg.max_runtime_ms,
            tick_interval_ms: at_cfg.tick_interval_ms,
        },
        initial_total_memories: seed.total_memories,
        initial_total_corrections: seed.total_corrections,
        training_config: archon_pipeline::learning::gnn::trainer::TrainingConfig {
            learning_rate: train_cfg.learning_rate,
            batch_size: train_cfg.batch_size,
            max_epochs: train_cfg.max_epochs,
            early_stopping_patience: train_cfg.early_stopping_patience,
            validation_split: train_cfg.validation_split,
            ewc_lambda: train_cfg.ewc_lambda,
            margin: train_cfg.margin,
            triplet_loss_coefficient: train_cfg.triplet_loss_coefficient,
            max_gradient_norm: train_cfg.max_gradient_norm,
            max_triplets_per_run: train_cfg.max_triplets_per_run,
            max_runtime_ms: train_cfg.max_runtime_ms,
            ..Default::default()
        },
        gnn_input_dim: gnn_cfg.input_dim,
        gnn_output_dim: gnn_cfg.output_dim,
        gnn_num_layers: gnn_cfg.num_layers,
        gnn_attention_heads: gnn_cfg.attention_heads,
        gnn_max_nodes: gnn_cfg.max_nodes,
        gnn_use_residual: gnn_cfg.use_residual,
        gnn_use_layer_norm: gnn_cfg.use_layer_norm,
        gnn_activation: gnn_cfg.activation.clone(),
        gnn_weight_seed: gnn_cfg.weight_seed,
    };
    let auto_trainer =
        archon_pipeline::learning::gnn::auto_trainer_runtime::build_and_spawn_auto_trainer(
            params,
            Arc::clone(db),
        );
    if auto_trainer.is_some() {
        tracing::info!(
            interval_ms = at_cfg.tick_interval_ms,
            throttle_ms = at_cfg.min_throttle_ms,
            first_run_threshold = at_cfg.first_run_threshold,
            seeded_memories = seed.total_memories,
            seeded_corrections = seed.total_corrections,
            "GNN auto-trainer spawned"
        );
    }
    auto_trainer
}
