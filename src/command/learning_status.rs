//! v0.1.23: /learning-status slash command handler.
//!
//! Reports status of all 8 learning subsystems: AutoCapture, AutoExtraction,
//! SONA, DESC, GNN, CausalMemory, ShadowVector, ReasoningBank, + Reflexion.
//!
//! v0.1.25: `/learning-status retrain` kicks off a synchronous GNN training run.

use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) struct LearningStatusHandler;

impl CommandHandler for LearningStatusHandler {
    fn description(&self) -> &str {
        "Report status of all learning subsystems (SONA, DESC, ReasoningBank, etc.)"
    }

    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> anyhow::Result<()> {
        if args.first().map(|s| s.as_str()) == Some("retrain") {
            return self.execute_retrain(ctx);
        }
        self.execute_status(ctx)
    }
}

impl LearningStatusHandler {
    fn execute_status(&self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        // Reference: archon-pipeline/src/learning/gnn/auto_trainer.rs::status.
        // Read live loop state if the AutoTrainer Arc is wired through. If not
        // wired (older session, build without learning DB, kill-switch), fall
        // back to config-claimed state and label it explicitly so the operator
        // can tell the difference.
        let live = ctx.auto_trainer.as_ref().map(|at| at.status());

        let status = match archon_core::config::load_config() {
            Ok(config) => {
                let at = &config.learning.gnn.auto_trainer;
                let gnn_line = if config.learning.gnn.enabled {
                    if let Some(snap) = &live {
                        format!(
                            "RUNNING — {} runs, {} mem / {} corr since last train, {} (training_in_progress={})",
                            snap.training_count,
                            snap.memories_since_last_train,
                            snap.corrections_since_last_train,
                            snap.seconds_since_last_train
                                .map(|s| format!("last {}s ago", s))
                                .unwrap_or_else(|| "never trained".into()),
                            snap.training_in_progress,
                        )
                    } else if at.enabled {
                        format!(
                            "config: enabled (every {}s, throttle {}min, triggers: {} mem / {} corr / {}h elapsed) — NOT spawned (no learning DB or kill-switch active)",
                            at.tick_interval_ms / 1000,
                            at.min_throttle_ms / 60_000,
                            at.trigger_new_memories,
                            at.trigger_corrections,
                            at.trigger_elapsed_ms / 3_600_000,
                        )
                    } else {
                        "OFF (auto_trainer.enabled=false in config)".to_string()
                    }
                } else {
                    "OFF (gnn.enabled=false in config)".to_string()
                };

                format!(
                    "## Learning Systems Status (v0.1.26)\n\
                     \n\
                     | Subsystem         | Status  |\n\
                     |-------------------|---------|\n\
                     | SONA              | {} |\n\
                     | DESC              | {} |\n\
                     | GNN               | {} |\n\
                     | GNN Auto-Trainer  | {} |\n\
                     | Causal Memory     | {} |\n\
                     | Shadow Vector     | {} |\n\
                     | Reasoning Bank    | {} |\n\
                     | AutoCapture       | {} |\n\
                     | AutoExtraction    | {} |\n\
                     | Reflexion         | {} |\n\
                     \n\
                     AutoExtraction interval: every {} turns.\n\
                     Reflexion max failures per agent: {}.",
                    on_off(config.learning.sona.enabled),
                    on_off(config.learning.desc.enabled),
                    on_off(config.learning.gnn.enabled),
                    gnn_line,
                    on_off(config.learning.causal_memory.enabled),
                    on_off(config.learning.shadow_vector.enabled),
                    on_off(config.learning.reasoning_bank.enabled),
                    on_off(config.memory.auto_capture.enabled),
                    on_off(config.memory.auto_extraction.enabled),
                    on_off(config.learning.reflexion.enabled),
                    config.memory.auto_extraction.every_n_turns,
                    config.learning.reflexion.max_per_agent,
                )
            }
            Err(e) => format!(
                "## Learning Systems Status (v0.1.26)\n\nConfig unavailable: {e}\n\n\
                 All learning subsystems are configured via `~/.archon/config.toml`."
            ),
        };

        let _ = ctx.tui_tx.send(TuiEvent::TextDelta(status));
        Ok(())
    }

    fn execute_retrain(&self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        let db = match &ctx.cozo_db {
            Some(db) => db,
            None => {
                let _ = ctx.tui_tx.send(TuiEvent::TextDelta(
                    "## GNN Retrain — ERROR\n\nCozoDB learning store is not available.\n\
                     Check that the learning database file exists and is writable."
                        .to_string(),
                ));
                return Ok(());
            }
        };

        // Ensure schemas exist
        archon_pipeline::learning::schema::initialize_learning_schemas(db)
            .map_err(|e| anyhow::anyhow!("Schema init failed: {e}"))?;

        // Load config for GNN model + training params
        let config = archon_core::config::load_config().unwrap_or_default();
        let gnn_cfg = &config.learning.gnn;
        let train_cfg_val = &gnn_cfg.training;

        // Query trajectories — shared with the auto-trainer's sample provider.
        // Reference: archon-pipeline/src/learning/gnn/auto_trainer_runtime.rs.
        let trajectories: Vec<archon_pipeline::learning::gnn::loss::TrajectoryWithFeedback> =
            match archon_pipeline::learning::gnn::auto_trainer_runtime::query_trajectories_for_training(db) {
                Ok(trajs) => trajs,
                Err(e) => {
                    let _ = ctx.tui_tx.send(TuiEvent::TextDelta(format!(
                        "## GNN Retrain — ERROR\n\nFailed to query trajectories: {e}"
                    )));
                    return Ok(());
                }
            };

        if trajectories.len() < 3 {
            let _ = ctx.tui_tx.send(TuiEvent::TextDelta(format!(
                "## GNN Retrain — SKIPPED\n\nNot enough trajectories with quality scores.\n\
                 Found {} trajectory(s); need at least 3 to build triplets.\n\
                 Have a conversation with quality feedback first.",
                trajectories.len()
            )));
            return Ok(());
        }

        // Build GNN components
        let gnn_model_config = archon_pipeline::learning::gnn::GnnConfig::default();
        let cache_config = archon_pipeline::learning::gnn::cache::CacheConfig::default();
        let weight_seed = if gnn_cfg.weight_seed == 0 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        } else {
            gnn_cfg.weight_seed
        };
        let enhancer = archon_pipeline::learning::gnn::GnnEnhancer::with_in_memory_weights(
            gnn_model_config,
            cache_config,
            weight_seed,
        );

        let weight_store = std::sync::Arc::new(
            archon_pipeline::learning::gnn::weights::WeightStore::new(std::sync::Arc::clone(db)),
        );

        let training_config = archon_pipeline::learning::gnn::trainer::TrainingConfig {
            learning_rate: train_cfg_val.learning_rate,
            batch_size: train_cfg_val.batch_size,
            max_epochs: train_cfg_val.max_epochs,
            early_stopping_patience: train_cfg_val.early_stopping_patience,
            validation_split: train_cfg_val.validation_split,
            ewc_lambda: train_cfg_val.ewc_lambda,
            margin: train_cfg_val.margin,
            max_gradient_norm: train_cfg_val.max_gradient_norm,
            max_triplets_per_run: train_cfg_val.max_triplets_per_run,
            max_runtime_ms: train_cfg_val.max_runtime_ms,
            ..Default::default()
        };

        let mut trainer = archon_pipeline::learning::gnn::trainer::GnnTrainer::new(
            training_config,
            Some(std::sync::Arc::clone(&weight_store)),
        );

        let weight_version_before = weight_store.current_version();

        // Run training (synchronous — blocks TUI input loop)
        let start = std::time::Instant::now();
        let outcome = trainer.train(&enhancer, &trajectories, None);
        let elapsed_ms = start.elapsed().as_millis();

        let weight_version_after = weight_store.current_version();
        let rolled_back =
            outcome.final_loss > outcome.initial_loss * 1.1 || outcome.final_loss.is_nan();

        // Write training run record
        let run_id = uuid::Uuid::new_v4().to_string();
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        let insert_run = format!(
            "?[run_id, started_at_ms, completed_at_ms, trigger_reason, samples_processed, \
             epochs_completed, final_loss, best_loss, weight_version_before, weight_version_after, \
             rolled_back, error] <- [\
             [\"{run_id}\", {now_ms}, {now_ms}, \"manual\", {samples}, {epochs}, \
             {final_loss}, {best_loss}, {version_before}, {version_after}, {rolled_back}, null]] \
             :put gnn_training_runs {{ run_id => started_at_ms, completed_at_ms, trigger_reason, \
             samples_processed, epochs_completed, final_loss, best_loss, \
             weight_version_before, weight_version_after, rolled_back, error }}",
            samples = outcome.samples_processed,
            epochs = outcome.epochs_completed,
            final_loss = outcome.final_loss,
            best_loss = outcome.best_loss,
            version_before = weight_version_before,
            version_after = weight_version_after,
            rolled_back = rolled_back,
        );
        let _ = db.run_script(
            &insert_run,
            Default::default(),
            cozo::ScriptMutability::Mutable,
        );

        // Build outcome table
        let verdict = if rolled_back {
            "ROLLED BACK (loss degraded or NaN)"
        } else if outcome.final_loss < outcome.initial_loss {
            "OK (loss improved)"
        } else {
            "OK (no degradation)"
        };

        let validation_str = outcome
            .validation_loss
            .map(|v| format!("{:.6}", v))
            .unwrap_or_else(|| "N/A".to_string());

        let report = format!(
            "## GNN Retrain — Complete\n\
             \n\
             | Metric              | Value              |\n\
             |---------------------|--------------------|\n\
             | Epochs              | {epochs:<18} |\n\
             | Batches             | {batches:<18} |\n\
             | Samples             | {samples:<18} |\n\
             | Initial Loss        | {init_loss:<18.6} |\n\
             | Final Loss          | {final_loss:<18.6} |\n\
             | Best Loss           | {best_loss:<18.6} |\n\
             | Validation Loss     | {val_loss:<18} |\n\
             | Weight Ver. Before  | {ver_before:<18} |\n\
             | Weight Ver. After   | {ver_after:<18} |\n\
             | Duration            | {elapsed_ms} ms          |\n\
             | Early Stop          | {early_stop:<18} |\n\
             | Verdict             | {verdict} |\n\
             \n\
             Run ID: `{run_id}`",
            epochs = outcome.epochs_completed,
            batches = outcome.batches_processed,
            samples = outcome.samples_processed,
            init_loss = outcome.initial_loss,
            final_loss = outcome.final_loss,
            best_loss = outcome.best_loss,
            val_loss = validation_str,
            ver_before = weight_version_before,
            ver_after = weight_version_after,
            early_stop = outcome.stopped_early,
            verdict = verdict,
        );

        let _ = ctx.tui_tx.send(TuiEvent::TextDelta(report));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn on_off(enabled: bool) -> &'static str {
    if enabled { "enabled" } else { "disabled" }
}

// `query_trajectories` was duplicated in this file and in auto_trainer_runtime;
// the shared implementation lives at
// `archon-pipeline/src/learning/gnn/auto_trainer_runtime.rs::query_trajectories_for_training`.
// Keeping a single source of truth means the auto-trainer's sample provider
// and the manual `/learning-status retrain` command always agree.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::CommandHandler;
    use crate::command::test_support::*;
    use archon_tui::app::TuiEvent;

    #[test]
    fn learning_status_smoke_emits_text_delta() {
        let (mut ctx, mut rx) = CtxBuilder::new().build();
        LearningStatusHandler
            .execute(&mut ctx, &[])
            .expect("execute must succeed");
        let events = drain_tui_events(&mut rx);
        let has_table = events.iter().any(|e| match e {
            TuiEvent::TextDelta(s) => s.contains("SONA") && s.contains("Learning Systems Status"),
            _ => false,
        });
        assert!(has_table, "must emit learning status table");
    }

    #[test]
    fn learning_status_handler_has_description() {
        let desc = LearningStatusHandler.description();
        assert!(
            desc.contains("learning"),
            "description must mention learning, got: {desc}"
        );
    }
}
