//! Game-theory pipeline execution and replay for `/gametheory`.

use anyhow::Result;
use archon_pipeline::gametheory;
use archon_pipeline::runner::LlmClient;
use archon_tui::app::TuiEvent;

use crate::command::gametheory_inspect;
use crate::command::pipeline_support::build_interactive_learning_stack;
use crate::command::registry::CommandContext;

use super::{emit, emit_db, open_db, render_usage_line};

pub(super) fn start_run(ctx: &mut CommandContext, args: &[String]) -> Result<()> {
    let kb_pack_id = parse_kb(args)?;
    let situation_args = args_without_flag_value(args, "--kb");
    if situation_args.is_empty() {
        return emit(ctx, render_usage_line("run requires <situation>"));
    }

    let situation = situation_args.join(" ");
    let tui_tx = ctx.tui_tx.clone();
    let llm = ctx.llm_adapter.clone();
    let loaded_config = archon_core::config::load_config().ok();
    let mut learning = loaded_config.as_ref().and_then(|config| {
        build_interactive_learning_stack(config, ctx.cozo_db.clone(), ctx.auto_trainer.clone())
    });
    let kb_note = kb_pack_id
        .as_deref()
        .map(|kb| format!(" using KB `{kb}`"))
        .unwrap_or_default();
    emit(
        ctx,
        format!("Starting game-theory run{kb_note} for: {situation}\n"),
    )?;

    archon_observability::spawn_named("gametheory-slash-run", async move {
        let result = async {
            let db = open_db()?;
            let llm_ref = llm.as_ref().map(|arc| arc.as_ref() as &dyn LlmClient);
            gametheory::run_full_pipeline_with_learning_options(
                &db,
                &situation,
                None,
                llm_ref,
                gametheory::GameTheoryMemoryContext::default(),
                gametheory::GameTheoryRunOptions {
                    kb_pack_id,
                    ..gametheory::GameTheoryRunOptions::default()
                },
                learning.as_mut(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
        }
        .await;

        let msg = match result {
            Ok(result) => format!(
                "Game-theory run complete: {} status={} specialists={} report_words={}\n",
                result.run_id,
                result.status,
                result.specialist_count,
                result.report.split_whitespace().count()
            ),
            Err(err) => format!("Game-theory run failed: {err}\n"),
        };
        let _ = tui_tx.send_async(TuiEvent::TextDelta(msg)).await;
    });
    Ok(())
}

pub(super) fn start_classify_only(ctx: &mut CommandContext, args: &[String]) -> Result<()> {
    if args.is_empty() {
        return emit(ctx, render_usage_line("classify-only requires <situation>"));
    }

    let situation = args.join(" ");
    let tui_tx = ctx.tui_tx.clone();
    let llm = ctx.llm_adapter.clone();
    let loaded_config = archon_core::config::load_config().ok();
    let mut learning = loaded_config.as_ref().and_then(|config| {
        build_interactive_learning_stack(config, ctx.cozo_db.clone(), ctx.auto_trainer.clone())
    });
    emit(
        ctx,
        format!("Classifying game-theory situation: {situation}\n"),
    )?;

    archon_observability::spawn_named("gametheory-slash-classify", async move {
        let result = async {
            let db = open_db()?;
            let llm_ref = llm.as_ref().map(|arc| arc.as_ref() as &dyn LlmClient);
            let fingerprint = gametheory::classify_with_learning(
                &db,
                &situation,
                llm_ref,
                learning.as_mut(),
            )
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(format!(
                "Game-theory classification persisted: run_id={} primary_family={} timing={} horizon={}\n",
                fingerprint.run_id,
                fingerprint.primary_family,
                fingerprint.timing.value,
                fingerprint.horizon.value
            ))
        }
        .await;

        let msg =
            result.unwrap_or_else(|err: anyhow::Error| format!("Classification failed: {err}\n"));
        let _ = tui_tx.send_async(TuiEvent::TextDelta(msg)).await;
    });
    Ok(())
}

pub(super) fn start_replay(ctx: &mut CommandContext, args: &[String]) -> Result<()> {
    let Some(run_id) = args.first() else {
        return emit(ctx, render_usage_line("replay requires <run-id>"));
    };
    let rerun_specialist = parse_rerun_specialist(args)?;
    let reclassify = args.iter().any(|arg| arg == "--reclassify");
    if reclassify && rerun_specialist.is_some() {
        anyhow::bail!("--reclassify and --rerun-specialist cannot be combined");
    }

    if reclassify || rerun_specialist.is_some() {
        return start_async_replay(ctx, run_id.clone(), reclassify, rerun_specialist);
    }

    emit_db(ctx, |db| {
        let routing = gametheory::replay_routing_from_stored_fingerprint(db, run_id, None)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        Ok(format!(
            "Replay routing for {run_id}: enabled={} skipped={}\n",
            routing.enabled_specialists.len(),
            routing.skipped_specialists.len()
        ))
    })
}

fn start_async_replay(
    ctx: &mut CommandContext,
    run_id: String,
    reclassify: bool,
    rerun_specialist: Option<String>,
) -> Result<()> {
    let tui_tx = ctx.tui_tx.clone();
    let llm = ctx.llm_adapter.clone();
    let loaded_config = archon_core::config::load_config().ok();
    let mut learning = loaded_config.as_ref().and_then(|config| {
        build_interactive_learning_stack(config, ctx.cozo_db.clone(), ctx.auto_trainer.clone())
    });
    emit(ctx, format!("Starting game-theory replay for {run_id}\n"))?;

    archon_observability::spawn_named("gametheory-slash-replay", async move {
        let result = async {
            let db = open_db()?;
            if reclassify {
                let Some(situation) = gametheory_inspect::load_run_situation(&db, &run_id)? else {
                    anyhow::bail!("run not found: {run_id}");
                };
                let llm_ref = llm.as_ref().map(|arc| arc.as_ref() as &dyn LlmClient);
                let result = gametheory::run_full_pipeline_with_learning_options(
                    &db,
                    &situation,
                    None,
                    llm_ref,
                    gametheory::GameTheoryMemoryContext::default(),
                    gametheory::GameTheoryRunOptions::default(),
                    learning.as_mut(),
                )
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
                return Ok(format!(
                    "Replay reclassified {run_id} as new run {} status={}\n",
                    result.run_id, result.status
                ));
            }

            let Some(agent_key) = rerun_specialist else {
                anyhow::bail!("internal replay error: missing rerun specialist");
            };
            let llm_ref = llm.as_ref().map(|arc| arc.as_ref() as &dyn LlmClient);
            let result = gametheory::replay_single_specialist_with_learning(
                &db,
                &run_id,
                &agent_key,
                llm_ref,
                gametheory::GameTheoryMemoryContext::default(),
                gametheory::GameTheoryRunOptions::default(),
                learning.as_mut(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(format!(
                "Replay specialist for {}: agent={} status={} cost=${:.6}\n",
                result.run_id, result.agent_key, result.status, result.cost_usd
            ))
        }
        .await;

        let msg = result.unwrap_or_else(|err: anyhow::Error| format!("Replay failed: {err}\n"));
        let _ = tui_tx.send_async(TuiEvent::TextDelta(msg)).await;
    });
    Ok(())
}

fn parse_kb(args: &[String]) -> Result<Option<String>> {
    for (idx, arg) in args.iter().enumerate() {
        if arg == "--kb" {
            let Some(value) = args.get(idx + 1) else {
                anyhow::bail!("--kb requires a knowledge-pack id");
            };
            return Ok(Some(value.clone()));
        }
        if let Some(value) = arg.strip_prefix("--kb=") {
            return Ok(Some(value.to_string()));
        }
    }
    Ok(None)
}

fn args_without_flag_value(args: &[String], flag: &str) -> Vec<String> {
    let mut cleaned = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == flag {
            skip_next = true;
            continue;
        }
        if arg.starts_with(&format!("{flag}=")) {
            continue;
        }
        cleaned.push(arg.clone());
    }
    cleaned
}

fn parse_rerun_specialist(args: &[String]) -> Result<Option<String>> {
    let Some(index) = args.iter().position(|arg| arg == "--rerun-specialist") else {
        return Ok(None);
    };
    let Some(value) = args.get(index + 1) else {
        anyhow::bail!("--rerun-specialist requires an agent key");
    };
    Ok(Some(value.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gametheory_run_kb_args_are_parsed_out_of_situation() {
        let args = vec![
            "Assess".to_string(),
            "marketplace".to_string(),
            "--kb".to_string(),
            "policy-pack".to_string(),
        ];

        assert_eq!(parse_kb(&args).unwrap().as_deref(), Some("policy-pack"));
        assert_eq!(
            args_without_flag_value(&args, "--kb"),
            vec!["Assess".to_string(), "marketplace".to_string()]
        );
    }

    #[test]
    fn test_gametheory_run_kb_equals_arg_is_parsed_out_of_situation() {
        let args = vec![
            "Assess".to_string(),
            "--kb=policy-pack".to_string(),
            "marketplace".to_string(),
        ];

        assert_eq!(parse_kb(&args).unwrap().as_deref(), Some("policy-pack"));
        assert_eq!(
            args_without_flag_value(&args, "--kb"),
            vec!["Assess".to_string(), "marketplace".to_string()]
        );
    }
}
