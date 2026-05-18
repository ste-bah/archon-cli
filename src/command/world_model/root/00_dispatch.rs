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
        WorldAction::TrainJepa {
            candidate,
            max_runtime_ms,
        } => {
            println!(
                "{}",
                candidate::render_train_jepa(
                    config,
                    &world_model_root()?,
                    *candidate,
                    *max_runtime_ms
                )?
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
        WorldAction::EvalJepa {
            candidate_id,
            full,
            background,
            resume,
            backend,
            no_cache,
        } => {
            println!(
                "{}",
                candidate::render_eval_jepa_with_options(
                    config,
                    &world_model_root()?,
                    candidate_id,
                    *full,
                    *background,
                    resume.clone(),
                    backend.clone(),
                    *no_cache,
                )?
            );
            Ok(())
        }
        WorldAction::EvalJepaStatus { run_id } => {
            println!(
                "{}",
                candidate::render_eval_jepa_status(&world_model_root()?, run_id)?
            );
            Ok(())
        }
        WorldAction::EvalJepaRuns { limit } => {
            println!(
                "{}",
                candidate::render_eval_jepa_runs(&world_model_root()?, *limit)?
            );
            Ok(())
        }
        WorldAction::EvalJepaCancel { run_id } => {
            println!(
                "{}",
                candidate::render_eval_jepa_cancel(&world_model_root()?, run_id)?
            );
            Ok(())
        }
        WorldAction::InspectJepa { candidate_id } => {
            println!(
                "{}",
                candidate::render_inspect_jepa(&world_model_root()?, candidate_id)?
            );
            Ok(())
        }
        WorldAction::CompareRepresentations {
            baseline,
            candidate,
        } => {
            println!(
                "{}",
                candidate::render_compare_representations(
                    config,
                    &world_model_root()?,
                    baseline,
                    candidate
                )?
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
        WorldAction::PromoteJepa { model_id } => {
            println!(
                "{}",
                candidate::render_promote_jepa(&world_model_root()?, model_id, config)?
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
        WorldAction::Guard { action } => {
            println!("{}", render_guard_command(action, config)?);
            Ok(())
        }
    }
}

