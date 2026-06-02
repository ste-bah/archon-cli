use std::path::{Path, PathBuf};
use std::sync::Arc;

use archon_cognitive::{CognitiveDaemon, CognitiveError, DaemonJob, DaemonJobReport};
use archon_core::config::ArchonConfig;
use archon_memory::{MemoryTrait, MemoryType, SearchFilter};
use archon_pipeline::learning::gnn::auto_trainer_one_shot::DurableTrainerSnapshot;

use crate::command::cognitive_daemon_learning_ledger as ledger;

const WORLD_JOB: &str = "world_model_trainer";
const GNN_JOB: &str = "gnn_auto_trainer";

pub(crate) fn add_learning_jobs<'a>(
    daemon: &mut CognitiveDaemon<'a>,
    config: ArchonConfig,
    cwd: PathBuf,
    cognitive_root: PathBuf,
) {
    if world_job_enabled(&config) {
        daemon.add_job(WorldModelTrainerJob {
            config: config.clone(),
            cognitive_root: cognitive_root.clone(),
        });
    }
    if gnn_job_enabled(&config) {
        daemon.add_job(GnnAutoTrainerJob {
            config,
            cwd,
            cognitive_root,
        });
    }
}

#[cfg(test)]
pub(crate) fn enabled_job_names(config: &ArchonConfig) -> Vec<&'static str> {
    let mut names = Vec::new();
    if world_job_enabled(config) {
        names.push(WORLD_JOB);
    }
    if gnn_job_enabled(config) {
        names.push(GNN_JOB);
    }
    names
}

pub(crate) fn render_recent_summary(cognitive_root: &Path) -> String {
    ledger::render_summary(cognitive_root)
}

fn world_job_enabled(config: &ArchonConfig) -> bool {
    config.learning.world_model.enabled && config.learning.world_model.auto_trainer.enabled
}

fn gnn_job_enabled(config: &ArchonConfig) -> bool {
    config.learning.gnn.enabled && config.learning.gnn.auto_trainer.enabled
}

struct WorldModelTrainerJob {
    config: ArchonConfig,
    cognitive_root: PathBuf,
}

impl DaemonJob for WorldModelTrainerJob {
    fn name(&self) -> &'static str {
        WORLD_JOB
    }

    fn run(&mut self) -> Result<DaemonJobReport, CognitiveError> {
        match crate::command::world_model::run_daemon_trainer_tick(&self.config) {
            Ok(summary) => {
                let event = ledger::LearningDaemonEvent::new(self.name(), "ok", &summary);
                append_event(&self.cognitive_root, &event);
                Ok(report(self.name(), true, summary))
            }
            Err(error) => {
                let summary = retry_summary(error.to_string());
                let event = ledger::LearningDaemonEvent::new(self.name(), "failed", &summary);
                append_event(&self.cognitive_root, &event);
                Ok(report(self.name(), false, summary))
            }
        }
    }
}

struct GnnAutoTrainerJob {
    config: ArchonConfig,
    cwd: PathBuf,
    cognitive_root: PathBuf,
}

impl DaemonJob for GnnAutoTrainerJob {
    fn name(&self) -> &'static str {
        GNN_JOB
    }

    fn run(&mut self) -> Result<DaemonJobReport, CognitiveError> {
        let outcome = self.run_inner();
        let event = event_from_gnn_outcome(self.name(), &outcome);
        append_event(&self.cognitive_root, &event);
        Ok(report(self.name(), outcome.ok, outcome.summary))
    }
}

impl GnnAutoTrainerJob {
    fn run_inner(&self) -> GnnJobOutcome {
        let db = match open_learning_db(&self.cwd) {
            Ok(db) => db,
            Err(error) => return GnnJobOutcome::failed(retry_summary(error.to_string())),
        };
        let memory = match open_memory_graph(&self.config) {
            Ok(memory) => memory,
            Err(error) => return GnnJobOutcome::failed(retry_summary(error.to_string())),
        };
        let counts = durable_memory_counts(&memory);
        let stats = training_run_stats(&db);
        let checkpoint = ledger::latest_count_checkpoint(&self.cognitive_root, self.name());
        let snapshot = DurableTrainerSnapshot {
            total_memories: counts.0,
            total_corrections: counts.1,
            memories_at_last_train: checkpoint.map(|counts| counts.0).unwrap_or(0),
            corrections_at_last_train: checkpoint.map(|counts| counts.1).unwrap_or(0),
            training_count: stats.training_count,
            last_attempt_elapsed_ms: stats
                .last_completed_ms
                .and_then(|last| now_ms().checked_sub(last)),
        };
        let params = build_params(&self.config);
        let report = archon_pipeline::learning::gnn::auto_trainer_one_shot::run_auto_trainer_once(
            params, db, snapshot,
        );
        let status = gnn_status(&report);
        GnnJobOutcome {
            ok: status != "failed",
            status,
            summary: report.summary,
            trained: report.trained,
            run_id: report.run_id,
            total_memories: Some(counts.0),
            total_corrections: Some(counts.1),
        }
    }
}

struct GnnJobOutcome {
    ok: bool,
    status: String,
    summary: String,
    trained: bool,
    run_id: Option<String>,
    total_memories: Option<u64>,
    total_corrections: Option<u64>,
}

impl GnnJobOutcome {
    fn failed(summary: String) -> Self {
        Self {
            ok: false,
            status: "failed".into(),
            summary,
            trained: false,
            run_id: None,
            total_memories: None,
            total_corrections: None,
        }
    }
}

#[derive(Default)]
struct TrainingRunStats {
    training_count: u64,
    last_completed_ms: Option<u64>,
}

fn open_learning_db(cwd: &Path) -> anyhow::Result<Arc<cozo::DbInstance>> {
    let path =
        crate::command::store_paths::evidence_db_path_for_dir(cwd, &["ARCHON_LEARNING_DB_PATH"]);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let db = archon_learning::cozo_guard::open_sqlite_guarded(
        path.to_str().unwrap_or(""),
        "open daemon GNN learning db",
    )?;
    archon_pipeline::learning::schema::initialize_learning_schemas(&db)?;
    Ok(Arc::new(db))
}

fn open_memory_graph(config: &ArchonConfig) -> anyhow::Result<archon_memory::MemoryGraph> {
    let (_, db_path) = archon_memory::resolve_memory_paths(config.memory.db_path.as_deref());
    Ok(archon_memory::MemoryGraph::open(db_path)?)
}

fn durable_memory_counts(memory: &dyn MemoryTrait) -> (u64, u64) {
    let total = memory.memory_count().unwrap_or_default() as u64;
    let filter = SearchFilter {
        memory_type: Some(MemoryType::Correction),
        ..Default::default()
    };
    let corrections = memory
        .search_memories(&filter)
        .map(|rows| rows.len() as u64)
        .unwrap_or_default();
    (total, corrections)
}

fn training_run_stats(db: &cozo::DbInstance) -> TrainingRunStats {
    let count = db
        .run_script(
            "?[run_id] := *gnn_training_runs[run_id, _, _, _, _, _, _, _, _, _, _, _]",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .map(|result| result.rows.len() as u64)
        .unwrap_or_default();
    let last_completed_ms = db
        .run_script(
            "?[completed] := *gnn_training_runs[_, _, completed, _, _, _, _, _, _, _, _, _]\n\
             :order -completed\n\
             :limit 1",
            Default::default(),
            cozo::ScriptMutability::Immutable,
        )
        .ok()
        .and_then(|result| result.rows.first().and_then(|row| row[0].get_int()))
        .map(|value| value.max(0) as u64);
    TrainingRunStats {
        training_count: count,
        last_completed_ms,
    }
}

fn build_params(
    config: &ArchonConfig,
) -> archon_pipeline::learning::gnn::auto_trainer_runtime::AutoTrainerBuildParams {
    let at = &config.learning.gnn.auto_trainer;
    let gnn = &config.learning.gnn;
    let train = &gnn.training;
    archon_pipeline::learning::gnn::auto_trainer_runtime::AutoTrainerBuildParams {
        at_config: archon_pipeline::learning::gnn::auto_trainer::AutoTrainerConfig {
            enabled: at.enabled,
            min_throttle_ms: at.min_throttle_ms,
            trigger_new_memories: at.trigger_new_memories,
            trigger_elapsed_ms: at.trigger_elapsed_ms,
            trigger_corrections: at.trigger_corrections,
            first_run_threshold: at.first_run_threshold,
            max_runtime_ms: at.max_runtime_ms,
            tick_interval_ms: at.tick_interval_ms,
        },
        initial_total_memories: 0,
        initial_total_corrections: 0,
        training_config: archon_pipeline::learning::gnn::trainer::TrainingConfig {
            learning_rate: train.learning_rate,
            batch_size: train.batch_size,
            max_epochs: train.max_epochs,
            early_stopping_patience: train.early_stopping_patience,
            validation_split: train.validation_split,
            ewc_lambda: train.ewc_lambda,
            margin: train.margin,
            triplet_loss_coefficient: train.triplet_loss_coefficient,
            max_gradient_norm: train.max_gradient_norm,
            max_triplets_per_run: train.max_triplets_per_run,
            max_runtime_ms: train.max_runtime_ms,
            ..Default::default()
        },
        gnn_input_dim: gnn.input_dim,
        gnn_output_dim: gnn.output_dim,
        gnn_num_layers: gnn.num_layers,
        gnn_attention_heads: gnn.attention_heads,
        gnn_max_nodes: gnn.max_nodes,
        gnn_use_residual: gnn.use_residual,
        gnn_use_layer_norm: gnn.use_layer_norm,
        gnn_activation: gnn.activation.clone(),
        gnn_weight_seed: gnn.weight_seed,
    }
}

fn event_from_gnn_outcome(job: &str, outcome: &GnnJobOutcome) -> ledger::LearningDaemonEvent {
    let mut event = ledger::LearningDaemonEvent::new(job, &outcome.status, &outcome.summary);
    event.trained = outcome.trained;
    event.run_id = outcome.run_id.clone();
    event.total_memories = outcome.total_memories;
    event.total_corrections = outcome.total_corrections;
    event
}

fn gnn_status(
    report: &archon_pipeline::learning::gnn::auto_trainer_one_shot::OneShotTrainerReport,
) -> String {
    if report.trained {
        "trained"
    } else if report.no_data_reason.is_some() {
        "no_data"
    } else if report.failed {
        "failed"
    } else {
        "skipped"
    }
    .into()
}

fn report(name: &str, ok: bool, summary: String) -> DaemonJobReport {
    DaemonJobReport {
        name: name.into(),
        ok,
        summary,
    }
}

fn append_event(root: &Path, event: &ledger::LearningDaemonEvent) {
    if let Err(error) = ledger::append(root, event) {
        tracing::warn!(error = %error, job = %event.job, "learning daemon ledger append failed");
    }
}

fn retry_summary(error: String) -> String {
    if error.contains("database is locked") || error.contains("Cozo") {
        format!("{error}; deferred until next daemon tick")
    } else {
        error
    }
}

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
    fn enabled_job_names_reflect_config_gates() {
        let mut config = ArchonConfig::default();

        assert_eq!(enabled_job_names(&config), vec![WORLD_JOB, GNN_JOB]);

        config.learning.world_model.auto_trainer.enabled = false;
        assert_eq!(enabled_job_names(&config), vec![GNN_JOB]);

        config.learning.gnn.enabled = false;
        assert!(enabled_job_names(&config).is_empty());
    }

    #[test]
    fn retry_summary_marks_database_locks_as_deferred() {
        let summary = retry_summary("database is locked (code 5)".into());

        assert!(summary.contains("deferred until next daemon tick"));
    }
}
