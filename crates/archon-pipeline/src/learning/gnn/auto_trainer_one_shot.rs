use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use cozo::{DbInstance, ScriptMutability};

use crate::learning::gnn::auto_trainer::AutoTrainerConfig;
use crate::learning::gnn::auto_trainer_runtime::{
    AutoTrainerBuildParams, query_trajectories_for_training,
};
use crate::learning::gnn::cache::CacheConfig;
use crate::learning::gnn::trainer::{GnnTrainer, TrainingOutcome};
use crate::learning::gnn::triplets_loss::TripletBatch;
use crate::learning::gnn::weights::WeightStore;
use crate::learning::gnn::{GnnConfig, GnnEnhancer};

#[derive(Debug, Clone, Default)]
pub struct DurableTrainerSnapshot {
    pub total_memories: u64,
    pub total_corrections: u64,
    pub memories_at_last_train: u64,
    pub corrections_at_last_train: u64,
    pub training_count: u64,
    pub last_attempt_elapsed_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct OneShotTrainerReport {
    pub decision: String,
    pub trained: bool,
    pub failed: bool,
    pub no_data_reason: Option<String>,
    pub run_id: Option<String>,
    pub summary: String,
}

pub fn run_auto_trainer_once(
    params: AutoTrainerBuildParams,
    db: Arc<DbInstance>,
    snapshot: DurableTrainerSnapshot,
) -> OneShotTrainerReport {
    let decision = trainer_decision(&params.at_config, &snapshot);
    if !decision.should_train {
        return OneShotTrainerReport {
            decision: decision.reason.clone(),
            trained: false,
            failed: false,
            no_data_reason: None,
            run_id: None,
            summary: format!("skipped: {}", decision.reason),
        };
    }

    let started_ms = now_ms();
    let run_id = uuid::Uuid::new_v4().to_string();
    match run_training(params, Arc::clone(&db)) {
        Ok((outcome, version_before, version_after)) => {
            let completed_ms = now_ms();
            let no_data = outcome.data_sources.no_data_reason().map(str::to_string);
            let rolled_back = outcome.final_loss > outcome.initial_loss * 1.1
                || outcome.final_loss.is_nan()
                || outcome.cancelled;
            let _ = insert_training_run(
                &db,
                TrainingRunRecord {
                    run_id: &run_id,
                    started_ms,
                    completed_ms,
                    trigger_reason: &decision.reason,
                    outcome: &outcome,
                    version_before,
                    version_after,
                    rolled_back,
                    error: None,
                },
            );
            let summary = if let Some(reason) = &no_data {
                format!("no training data: {reason}")
            } else {
                format!(
                    "trained run={run_id} samples={} epochs={} loss {:.4}->{:.4}",
                    outcome.samples_processed,
                    outcome.epochs_completed,
                    outcome.initial_loss,
                    outcome.final_loss
                )
            };
            OneShotTrainerReport {
                decision: decision.reason,
                trained: no_data.is_none(),
                failed: false,
                no_data_reason: no_data,
                run_id: Some(run_id),
                summary,
            }
        }
        Err(error) => {
            let _ = insert_error_run(&db, &run_id, started_ms, &decision.reason, &error);
            OneShotTrainerReport {
                decision: decision.reason,
                trained: false,
                failed: true,
                no_data_reason: None,
                run_id: Some(run_id),
                summary: format!("training failed: {error}"),
            }
        }
    }
}

#[derive(Debug, Clone)]
struct TrainerDecision {
    should_train: bool,
    reason: String,
}

fn trainer_decision(
    config: &AutoTrainerConfig,
    snapshot: &DurableTrainerSnapshot,
) -> TrainerDecision {
    if !config.enabled {
        return skip("disabled");
    }
    if let Some(elapsed) = snapshot.last_attempt_elapsed_ms
        && elapsed < config.min_throttle_ms
    {
        return skip(format!("throttled {elapsed}/{}ms", config.min_throttle_ms));
    }

    let memories_since = snapshot
        .total_memories
        .saturating_sub(snapshot.memories_at_last_train);
    let corrections_since = snapshot
        .total_corrections
        .saturating_sub(snapshot.corrections_at_last_train);
    if snapshot.training_count == 0 {
        if config.trigger_corrections > 0 && corrections_since >= config.trigger_corrections {
            return train("corrections");
        }
        if snapshot.total_memories >= config.first_run_threshold {
            return train("first_run");
        }
        return skip(format!(
            "below first-run gate {}/{}",
            snapshot.total_memories, config.first_run_threshold
        ));
    }
    if memories_since >= config.trigger_new_memories {
        return train("new_memories");
    }
    if config.trigger_corrections > 0 && corrections_since >= config.trigger_corrections {
        return train("corrections");
    }
    if let Some(elapsed) = snapshot.last_attempt_elapsed_ms
        && elapsed >= config.trigger_elapsed_ms
    {
        return train("elapsed");
    }
    skip(format!(
        "gates closed mem={memories_since}/{} corr={corrections_since}/{}",
        config.trigger_new_memories, config.trigger_corrections
    ))
}

fn train(reason: &str) -> TrainerDecision {
    TrainerDecision {
        should_train: true,
        reason: reason.into(),
    }
}

fn skip(reason: impl Into<String>) -> TrainerDecision {
    TrainerDecision {
        should_train: false,
        reason: reason.into(),
    }
}

fn run_training(
    params: AutoTrainerBuildParams,
    db: Arc<DbInstance>,
) -> Result<(TrainingOutcome, i64, i64), String> {
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
    let seed = gnn_seed(params.gnn_weight_seed);
    let weight_store = Arc::new(WeightStore::new(Arc::clone(&db)));
    let enhancer = GnnEnhancer::new(
        gnn_config,
        CacheConfig::default(),
        seed,
        WeightStore::new(Arc::clone(&db)),
    );
    let samples = query_trajectories_for_training(&db, params.gnn_input_dim)?;
    let triplet_batch = hydrated_triplets(&db);
    let version_before = weight_store.current_version();
    let cancel = AtomicBool::new(false);
    let mut trainer = GnnTrainer::new(params.training_config, Some(Arc::clone(&weight_store)));
    let outcome = trainer.train_with_triplets(&enhancer, &samples, &triplet_batch, Some(&cancel));
    let version_after = weight_store.current_version();
    Ok((outcome, version_before, version_after))
}

fn hydrated_triplets(db: &DbInstance) -> TripletBatch {
    archon_meaning::list_hydrated_triplets(db, 256)
        .map(|triplets| TripletBatch { triplets })
        .unwrap_or_default()
}

fn gnn_seed(configured: u64) -> u64 {
    if configured == 0 {
        now_ms() / 1000
    } else {
        configured
    }
}

fn parse_activation(s: &str) -> crate::learning::gnn::math::ActivationType {
    use crate::learning::gnn::math::ActivationType;
    match s.to_lowercase().as_str() {
        "tanh" => ActivationType::Tanh,
        "sigmoid" => ActivationType::Sigmoid,
        "leakyrelu" | "leaky_relu" | "leaky-relu" => ActivationType::LeakyRelu,
        _ => ActivationType::Relu,
    }
}

struct TrainingRunRecord<'a> {
    run_id: &'a str,
    started_ms: u64,
    completed_ms: u64,
    trigger_reason: &'a str,
    outcome: &'a TrainingOutcome,
    version_before: i64,
    version_after: i64,
    rolled_back: bool,
    error: Option<&'a str>,
}

fn insert_training_run(db: &DbInstance, record: TrainingRunRecord<'_>) -> Result<(), String> {
    let mut params = std::collections::BTreeMap::new();
    params.insert("run_id".into(), cozo::DataValue::from(record.run_id));
    params.insert(
        "started".into(),
        cozo::DataValue::from(record.started_ms as i64),
    );
    params.insert(
        "completed".into(),
        cozo::DataValue::from(record.completed_ms as i64),
    );
    params.insert(
        "trigger".into(),
        cozo::DataValue::from(record.trigger_reason),
    );
    params.insert(
        "samples".into(),
        cozo::DataValue::from(record.outcome.samples_processed as i64),
    );
    params.insert(
        "epochs".into(),
        cozo::DataValue::from(record.outcome.epochs_completed as i64),
    );
    params.insert(
        "final_loss".into(),
        cozo::DataValue::from(record.outcome.final_loss as f64),
    );
    params.insert(
        "best_loss".into(),
        cozo::DataValue::from(record.outcome.best_loss as f64),
    );
    params.insert(
        "before".into(),
        cozo::DataValue::from(record.version_before),
    );
    params.insert("after".into(), cozo::DataValue::from(record.version_after));
    params.insert(
        "rolled_back".into(),
        cozo::DataValue::from(record.rolled_back),
    );
    params.insert(
        "error".into(),
        record
            .error
            .map(cozo::DataValue::from)
            .unwrap_or(cozo::DataValue::Null),
    );
    db.run_script(TRAINING_RUN_INSERT, params, ScriptMutability::Mutable)
        .map_err(|error| format!("insert gnn training run: {error}"))?;
    Ok(())
}

fn insert_error_run(
    db: &DbInstance,
    run_id: &str,
    started_ms: u64,
    trigger_reason: &str,
    error: &str,
) -> Result<(), String> {
    let outcome = TrainingOutcome {
        epochs_completed: 0,
        batches_processed: 0,
        samples_processed: 0,
        sona_samples_processed: 0,
        meaning_triplets_processed: 0,
        data_sources: Default::default(),
        initial_loss: 0.0,
        final_loss: 0.0,
        best_loss: 0.0,
        validation_loss: None,
        stopped_early: false,
        timed_out: false,
        cancelled: false,
        best_epoch: 0,
        best_val_loss: 0.0,
        best_train_loss: 0.0,
        epoch_metrics: Vec::new(),
    };
    insert_training_run(
        db,
        TrainingRunRecord {
            run_id,
            started_ms,
            completed_ms: now_ms(),
            trigger_reason,
            outcome: &outcome,
            version_before: 0,
            version_after: 0,
            rolled_back: true,
            error: Some(error),
        },
    )
}

const TRAINING_RUN_INSERT: &str = "
?[run_id, started_at_ms, completed_at_ms, trigger_reason, samples_processed, epochs_completed,
  final_loss, best_loss, weight_version_before, weight_version_after, rolled_back, error] <-
  [[$run_id, $started, $completed, $trigger, $samples, $epochs, $final_loss, $best_loss,
    $before, $after, $rolled_back, $error]]
:put gnn_training_runs { run_id => started_at_ms, completed_at_ms, trigger_reason,
  samples_processed, epochs_completed, final_loss, best_loss, weight_version_before,
  weight_version_after, rolled_back, error }
";

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_run_opens_when_memory_gate_is_met() {
        let cfg = AutoTrainerConfig {
            first_run_threshold: 3,
            ..Default::default()
        };
        let report = trainer_decision(
            &cfg,
            &DurableTrainerSnapshot {
                total_memories: 3,
                ..Default::default()
            },
        );
        assert!(report.should_train);
        assert_eq!(report.reason, "first_run");
    }

    #[test]
    fn throttle_closes_even_when_memory_gate_is_open() {
        let cfg = AutoTrainerConfig {
            min_throttle_ms: 1000,
            ..Default::default()
        };
        let report = trainer_decision(
            &cfg,
            &DurableTrainerSnapshot {
                total_memories: 100,
                training_count: 1,
                last_attempt_elapsed_ms: Some(10),
                ..Default::default()
            },
        );
        assert!(!report.should_train);
        assert!(report.reason.contains("throttled"));
    }
}
