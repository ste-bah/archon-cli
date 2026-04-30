//! Runtime wiring for the GNN auto-trainer — config → spawned background loop.
//!
//! Reference: `auto_trainer.rs` (lifecycle), `learning_status.rs::query_trajectories`
//! (sample query — moved here so both interactive session and pipeline command
//! can construct an AutoTrainer without duplicating CozoDB query logic).
//!
//! This module exists because v0.1.26 shipped the AutoTrainer infrastructure
//! but never wired it from the application's startup path:
//!   - `src/command/pipeline.rs:72` constructed `LearningIntegration::new(None,…)`
//!   - `src/session.rs` (interactive) never constructed AutoTrainer at all
//!   - `/learning-status` reported config-claimed state, not live loop state
//!
//! The `build_and_spawn_auto_trainer` helper closes that gap: given a config
//! and CozoDB handle, it builds enhancer + weight_store + training_config +
//! sample_provider and spawns the background loop. Returns the `Arc<AutoTrainer>`
//! so callers can pass it to LearningIntegration AND to slash-context for
//! live status reads.

use std::sync::Arc;

use cozo::DbInstance;

use crate::learning::gnn::auto_trainer::{AutoTrainer, AutoTrainerConfig};
use crate::learning::gnn::cache::CacheConfig;
use crate::learning::gnn::loss::TrajectoryWithFeedback;
use crate::learning::gnn::trainer::TrainingConfig;
use crate::learning::gnn::weights::WeightStore;
use crate::learning::gnn::{GnnConfig, GnnEnhancer};

// ---------------------------------------------------------------------------
// Sample provider — replaces the duplicated query in learning_status.rs
// ---------------------------------------------------------------------------

/// Query trajectories with quality scores from CozoDB.
///
/// Returns up to 512 trajectories with `quality > 0`, ordered by quality
/// descending. Used as the AutoTrainer sample provider AND by the manual
/// `/learning-status retrain` command.
pub fn query_trajectories_for_training(
    db: &DbInstance,
) -> Result<Vec<TrajectoryWithFeedback>, String> {
    let query = "
        ?[trajectory_id, quality] :=
            *trajectories[trajectory_id, _, _, _, _, _, quality, _, _, _, _, _],
            quality > 0.0
        :order -quality
        :limit 512
    ";

    let result = db
        .run_script(query, Default::default(), cozo::ScriptMutability::Immutable)
        .map_err(|e| format!("CozoDB query failed: {e}"))?;

    let mut trajectories = Vec::with_capacity(result.rows.len());
    for row in &result.rows {
        let trajectory_id = row[0].get_str().unwrap_or("unknown").to_string();
        let quality = row[1].get_float().unwrap_or(0.0) as f32;
        // Embedding storage is not yet wired into the trajectories schema —
        // the manual retrain command also uses a zero placeholder. When the
        // schema gains an embedding column, both call sites switch in lockstep.
        let embedding = vec![0.0f32; 1536];
        trajectories.push(TrajectoryWithFeedback {
            trajectory_id,
            embedding,
            quality,
        });
    }

    Ok(trajectories)
}

// ---------------------------------------------------------------------------
// Build + spawn from runtime config
// ---------------------------------------------------------------------------

/// Inputs needed to build the AutoTrainer from the application's startup state.
///
/// The application's config types live in `archon-core`; this struct decouples
/// the gnn module from `archon-core` so we can construct the training plumbing
/// here without a circular dep.
pub struct AutoTrainerBuildParams {
    pub at_config: AutoTrainerConfig,
    pub training_config: TrainingConfig,
    pub gnn_input_dim: usize,
    pub gnn_output_dim: usize,
    pub gnn_num_layers: usize,
    pub gnn_attention_heads: usize,
    pub gnn_max_nodes: usize,
    pub gnn_use_residual: bool,
    pub gnn_use_layer_norm: bool,
    pub gnn_activation: String,
    pub gnn_weight_seed: u64,
}

/// Build the AutoTrainer + dependencies and spawn the background loop.
///
/// Returns `None` if `at_config.enabled` is false (no spawn, no allocations
/// for unused state). Returns `Some(Arc<AutoTrainer>)` otherwise — the caller
/// is responsible for storing the Arc and calling `shutdown()` on application
/// exit.
///
/// The spawned loop calls `query_trajectories_for_training(db)` on each tick
/// when triggers fire.
pub fn build_and_spawn_auto_trainer(
    params: AutoTrainerBuildParams,
    db: Arc<DbInstance>,
) -> Option<Arc<AutoTrainer>> {
    if !params.at_config.enabled {
        return None;
    }

    let gnn_config = GnnConfig {
        input_dim: params.gnn_input_dim,
        output_dim: params.gnn_output_dim,
        num_layers: params.gnn_num_layers,
        attention_heads: params.gnn_attention_heads,
        dropout: 0.1,
        max_nodes: params.gnn_max_nodes,
        use_residual: params.gnn_use_residual,
        use_layer_norm: params.gnn_use_layer_norm,
        activation: parse_activation(&params.gnn_activation),
    };
    let cache_config = CacheConfig::default();
    let seed = if params.gnn_weight_seed == 0 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    } else {
        params.gnn_weight_seed
    };

    // Two WeightStores backed by the same DbInstance: one for the enhancer
    // (used during inference inside the training loop) and one passed into
    // the trainer for weight persistence. WeightStore::new is cheap; both
    // share state via the cloned Arc<DbInstance>.
    let enhancer_weights = WeightStore::new(Arc::clone(&db));
    let trainer_weights = Arc::new(WeightStore::new(Arc::clone(&db)));
    let enhancer = Arc::new(GnnEnhancer::new(
        gnn_config,
        cache_config,
        seed,
        enhancer_weights,
    ));

    // Sample provider closes over the shared db handle. Errors are logged
    // and converted to an empty sample set so the auto-trainer's tick loop
    // never panics on a transient query failure.
    let db_for_provider = Arc::clone(&db);
    let sample_provider: Arc<
        dyn Fn() -> Vec<TrajectoryWithFeedback> + Send + Sync,
    > = Arc::new(move || {
        match query_trajectories_for_training(&db_for_provider) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "AutoTrainer: sample provider query failed");
                Vec::new()
            }
        }
    });

    let at = Arc::new(AutoTrainer::new(params.at_config));
    at.spawn(
        enhancer,
        trainer_weights,
        params.training_config,
        sample_provider,
    );
    Some(at)
}

fn parse_activation(s: &str) -> crate::learning::gnn::math::ActivationType {
    // Reference: math.rs::ActivationType — supports Relu, Tanh, Sigmoid, LeakyRelu.
    use crate::learning::gnn::math::ActivationType;
    match s.to_lowercase().as_str() {
        "tanh" => ActivationType::Tanh,
        "sigmoid" => ActivationType::Sigmoid,
        "leakyrelu" | "leaky_relu" | "leaky-relu" => ActivationType::LeakyRelu,
        _ => ActivationType::Relu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_returns_none_when_disabled() {
        let db = Arc::new(cozo::DbInstance::new("mem", "", "").expect("mem db"));
        let params = AutoTrainerBuildParams {
            at_config: AutoTrainerConfig {
                enabled: false,
                ..Default::default()
            },
            training_config: TrainingConfig::default(),
            gnn_input_dim: 1536,
            gnn_output_dim: 1536,
            gnn_num_layers: 3,
            gnn_attention_heads: 12,
            gnn_max_nodes: 50,
            gnn_use_residual: true,
            gnn_use_layer_norm: true,
            gnn_activation: "relu".into(),
            gnn_weight_seed: 42,
        };
        assert!(build_and_spawn_auto_trainer(params, db).is_none());
    }

    #[test]
    fn parse_activation_defaults_to_relu_for_unknown() {
        // Reference: math.rs::ActivationType variants.
        use crate::learning::gnn::math::ActivationType;
        assert!(matches!(parse_activation("garbage"), ActivationType::Relu));
        assert!(matches!(parse_activation("RELU"), ActivationType::Relu));
        assert!(matches!(parse_activation("tanh"), ActivationType::Tanh));
        assert!(matches!(parse_activation("Sigmoid"), ActivationType::Sigmoid));
        assert!(matches!(parse_activation("leaky_relu"), ActivationType::LeakyRelu));
    }
}
