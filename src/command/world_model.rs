//! `archon world` CLI handlers.

use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::cli_args::WorldAction;

mod actions;
mod candidate;
mod embedding_runtime;
mod ingest_files;
mod labeling_runtime;
mod predict;
mod runtime;
mod status;
mod trainer_runtime;

pub(crate) use runtime::{
    record_provider_runtime_advisory, record_runtime_advisory,
    record_runtime_counterfactual_advice, record_runtime_outcome,
};
pub(super) use status::load_world_model_stats;
pub(crate) use status::render_world_status;
#[cfg(test)]
pub(super) use status::render_world_status_with_stats;
pub(crate) use trainer_runtime::schedule_dynamic_trainer_tick;

pub(crate) async fn handle_world_command(
    action: &WorldAction,
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
) -> Result<()> {
    match action {
        WorldAction::Status => {
            println!("{}", render_world_status(config));
            Ok(())
        }
        WorldAction::Ingest {
            session_id,
            backfill,
        } => {
            validate_ingest_args(session_id.as_deref(), *backfill)?;
            println!(
                "{}",
                render_ingest(config, env_vars, session_id.as_deref(), *backfill).await?
            );
            Ok(())
        }
        WorldAction::PredictNext {
            session_id,
            action_ref,
            summary,
        } => {
            println!(
                "{}",
                render_predict_next(config, session_id, action_ref, summary)?
            );
            Ok(())
        }
        WorldAction::ScoreActions { task, actions } => {
            println!(
                "{}",
                actions::render_score_actions(config, &world_model_root()?, task, actions)?
            );
            Ok(())
        }
        WorldAction::Explain { prediction_id } => {
            println!(
                "{}",
                actions::render_explain(&world_model_root()?, prediction_id)
            );
            Ok(())
        }
        WorldAction::RecordOutcome {
            prediction_id,
            actual_summary,
        } => {
            println!(
                "{}",
                predict::render_record_outcome(
                    config,
                    &world_model_root()?,
                    prediction_id,
                    actual_summary
                )?
            );
            Ok(())
        }
        WorldAction::Train {
            candidate,
            max_runtime_ms,
        } => {
            println!(
                "{}",
                candidate::render_train(config, &world_model_root()?, *candidate, *max_runtime_ms)?
            );
            Ok(())
        }
        WorldAction::TrainerTick {
            last_activity_age_ms,
            last_training_age_ms,
            battery_percent,
            unplugged,
        } => {
            println!(
                "{}",
                candidate::render_trainer_tick(
                    config,
                    &world_model_root()?,
                    *last_activity_age_ms,
                    *last_training_age_ms,
                    *battery_percent,
                    *unplugged
                )?
            );
            Ok(())
        }
        WorldAction::Eval { candidate_id } => {
            println!(
                "{}",
                candidate::render_eval(config, &world_model_root()?, candidate_id.as_deref())?
            );
            Ok(())
        }
        WorldAction::Promote { model_id } => {
            println!(
                "{}",
                candidate::render_promote(&world_model_root()?, model_id)?
            );
            Ok(())
        }
        WorldAction::Rollback { model_id } => {
            println!(
                "{}",
                candidate::render_rollback(&world_model_root()?, model_id)?
            );
            Ok(())
        }
    }
}

fn validate_ingest_args(session_id: Option<&str>, backfill: bool) -> Result<()> {
    match (session_id, backfill) {
        (Some(_), true) => bail!("use either a session id or --backfill, not both"),
        (None, false) => bail!("provide a session id or --backfill"),
        _ => Ok(()),
    }
}

async fn render_ingest(
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
    session_id: Option<&str>,
    backfill: bool,
) -> Result<String> {
    if backfill {
        render_backfill_ingest(config, env_vars).await
    } else {
        render_session_ingest(config, env_vars, session_id.unwrap_or_default()).await
    }
}

async fn render_session_ingest(
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
    session_id: &str,
) -> Result<String> {
    let labeler =
        labeling_runtime::WorldModelLabelingRuntime::from_config(config, env_vars).await?;
    let report = ingest_files::ingest_session(
        session_id,
        &sessions_dir()?,
        &world_model_root()?,
        &std::env::current_dir()?,
        retention_policy(config),
        &labeler,
    )
    .await?;

    Ok(format!(
        "World Model Ingest\n\
         ==================\n\
         Mode: session\n\
         Session: {session_id}\n\
         Files read: {}\n\
         Rows normalized: {}\n\
         Rows persisted:  {}\n\
         Cozo upserts:    {}\n\
         Warnings: {}\n\
         Ledger: {}\n\
         Store: {}\n\
         Sources: {}",
        report.files_read,
        report.rows_normalized,
        report.rows_persisted,
        report.cozo_rows,
        report.warnings,
        report.ledger_path.display(),
        report.db_path.display(),
        report.sources_summary()
    ))
}

async fn render_backfill_ingest(
    config: &archon_core::config::ArchonConfig,
    env_vars: &archon_core::env_vars::ArchonEnvVars,
) -> Result<String> {
    let sessions_dir = sessions_dir()?;
    let labeler =
        labeling_runtime::WorldModelLabelingRuntime::from_config(config, env_vars).await?;
    let report = ingest_files::ingest_backfill(
        &sessions_dir,
        &world_model_root()?,
        &std::env::current_dir()?,
        retention_policy(config),
        &labeler,
    )
    .await?;

    Ok(format!(
        "World Model Ingest\n\
         ==================\n\
         Mode: backfill\n\
         Files read: {}\n\
         Rows normalized: {}\n\
         Rows persisted:  {}\n\
         Cozo upserts:    {}\n\
         Warnings: {}\n\
         Ledger: {}\n\
         Store: {}\n\
         Sessions dir: {}\n\
         Sources: {}",
        report.files_read,
        report.rows_normalized,
        report.rows_persisted,
        report.cozo_rows,
        report.warnings,
        report.ledger_path.display(),
        report.db_path.display(),
        sessions_dir.display(),
        report.sources_summary()
    ))
}

fn retention_policy(
    config: &archon_core::config::ArchonConfig,
) -> archon_world_model::storage::RetentionPolicy {
    let retention = &config.learning.world_model.retention;
    archon_world_model::storage::RetentionPolicy {
        jsonl_rotate_bytes: retention.jsonl_rotate_mb.saturating_mul(1024 * 1024),
        raw_retention_days: retention.raw_retention_days,
    }
}

fn sessions_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("home directory unavailable"))?;
    Ok(home.join(".archon").join("sessions"))
}

fn world_model_root() -> Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("home directory unavailable"))?;
    Ok(home.join(".archon").join("world-model"))
}

fn open_world_model_store() -> Result<archon_world_model::storage::WorldModelStore> {
    archon_world_model::storage::WorldModelStore::open(world_model_root()?)
}

fn model_registry() -> Result<archon_world_model::registry::ModelRegistry> {
    archon_world_model::registry::ModelRegistry::open(world_model_root()?)
}

fn active_model_id() -> Result<Option<String>> {
    model_registry()?.active_model_id()
}

#[cfg(test)]
fn activity_jsonl_paths_under(root: &std::path::Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    if !root.exists() {
        return Ok(paths);
    }

    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let candidate = entry.path().join("activity").join("events.jsonl");
        if candidate.is_file() {
            paths.push(candidate);
        }
    }
    paths.sort();
    Ok(paths)
}

fn render_predict_next(
    config: &archon_core::config::ArchonConfig,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> Result<String> {
    let stats = match load_world_model_stats() {
        Ok(stats) => stats,
        Err(_) => {
            return Ok(render_unavailable_prediction(
                session_id,
                action_ref,
                "StoreUnavailable",
            ));
        }
    };
    let active_model_id = match active_model_id() {
        Ok(active_model_id) => active_model_id,
        Err(_) => {
            return Ok(render_unavailable_prediction(
                session_id,
                action_ref,
                "StoreUnavailable",
            ));
        }
    };
    Ok(render_predict_next_with_state(
        config,
        &world_model_root()?,
        stats,
        active_model_id,
        session_id,
        action_ref,
        summary,
    ))
}

fn render_unavailable_prediction(session_id: &str, action_ref: &str, reason: &str) -> String {
    format!(
        "World Model Prediction\n\
         ======================\n\
         Session: {session_id}\n\
         Action ref: {action_ref}\n\
         Unavailable: {reason}\n\
         Behavior: fail-open"
    )
}

fn render_predict_next_with_state(
    config: &archon_core::config::ArchonConfig,
    root: &std::path::Path,
    stats: archon_world_model::ColdStartStats,
    active_model_id: Option<String>,
    session_id: &str,
    action_ref: &str,
    summary: &str,
) -> String {
    if let Some(rendered) = predict::render_active_checkpoint_prediction(
        config,
        root,
        stats,
        active_model_id.clone(),
        session_id,
        action_ref,
        summary,
    ) {
        return rendered;
    }

    let advisor = archon_world_model::WorldAdvisor::new(
        archon_world_model::WorldAdvisorConfig {
            thresholds: cold_start_thresholds(config),
            active_model_id,
            training_in_progress: false,
        },
        stats,
    );
    let context = archon_world_model::WorldAdvisorContext {
        session_id: session_id.to_string(),
        action_ref: action_ref.to_string(),
        action_summary: summary.to_string(),
    };
    let decision = advisor.evaluate(&context);

    if let Some(prediction) = decision.prediction {
        format!(
            "World Model Prediction\n\
             ======================\n\
             Session: {session_id}\n\
             Action ref: {action_ref}\n\
             Model: {}\n\
             Prediction: {}",
            prediction.model_id, prediction.predicted_next_state_summary
        )
    } else {
        let reason = decision
            .unavailable
            .map(|event| format!("{:?}", event.reason))
            .unwrap_or_else(|| "Unknown".into());
        render_unavailable_prediction(session_id, action_ref, &reason)
    }
}

fn cold_start_thresholds(
    config: &archon_core::config::ArchonConfig,
) -> archon_world_model::ColdStartThresholds {
    let cold_start = &config.learning.world_model.cold_start;
    archon_world_model::ColdStartThresholds {
        min_rows: cold_start.min_rows,
        min_sessions: cold_start.min_sessions,
        min_observed_days: cold_start.min_observed_days,
    }
}

#[cfg(test)]
mod tests;
