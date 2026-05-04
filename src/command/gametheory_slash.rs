//! `/gametheory` slash-command umbrella.

use anyhow::Result;
use archon_pipeline::gametheory;
use archon_pipeline::runner::LlmClient;
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};
use cozo::{DbInstance, ScriptMutability};

use crate::command::gametheory_inspect;
use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) const GAMETHEORY_SUBCOMMANDS: &[&str] = &[
    "run",
    "classify-only",
    "status",
    "inspect",
    "inspect-fingerprint",
    "inspect-routing",
    "list-runs",
    "show",
    "replay",
    "list-agents",
    "specimens",
    "view",
];

pub(crate) struct GameTheorySlashHandler;

impl CommandHandler for GameTheorySlashHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        let subcommand = args.first().map(String::as_str).unwrap_or("");
        let rest = if args.is_empty() { &[] } else { &args[1..] };

        match subcommand {
            "" | "help" => emit(ctx, render_usage()),
            "run" => start_run(ctx, rest),
            "classify-only" => start_classify_only(ctx, rest),
            "status" => emit_db(ctx, |db| {
                gametheory_inspect::render_status(db, rest.first().map(String::as_str))
            }),
            "inspect" => match rest.first() {
                Some(artifact_id) => emit_db(ctx, |db| {
                    gametheory_inspect::render_inspect_artifact(db, artifact_id)
                }),
                None => emit(ctx, render_usage_line("inspect requires <artifact-id>")),
            },
            "inspect-fingerprint" => match rest.first() {
                Some(run_id) => emit_db(ctx, |db| {
                    gametheory_inspect::render_inspect_fingerprint(db, run_id)
                }),
                None => emit(
                    ctx,
                    render_usage_line("inspect-fingerprint requires <run-id>"),
                ),
            },
            "inspect-routing" => match rest.first() {
                Some(run_id) => emit_db(ctx, |db| {
                    gametheory_inspect::render_inspect_routing(db, run_id)
                }),
                None => emit(ctx, render_usage_line("inspect-routing requires <run-id>")),
            },
            "list-runs" => emit_db(ctx, gametheory_inspect::render_list_runs),
            "show" => match rest.first() {
                Some(run_id) => emit_db(ctx, |db| gametheory_inspect::render_show(db, run_id)),
                None => emit(ctx, render_usage_line("show requires <run-id>")),
            },
            "replay" => start_replay(ctx, rest),
            "list-agents" => emit(
                ctx,
                gametheory_inspect::render_list_agents(parse_tier(rest)?)?,
            ),
            "specimens" => emit_db(ctx, |db| render_specimens(db, rest)),
            "view" | "open" => emit_db_event(ctx, open_gametheory_rows_event),
            other => emit(
                ctx,
                render_usage_line(&format!("unknown subcommand `{other}`")),
            ),
        }
    }

    fn description(&self) -> &str {
        "Run and inspect the game-theory evidence pipeline"
    }
}

fn start_run(ctx: &mut CommandContext, args: &[String]) -> Result<()> {
    let kb_pack_id = parse_kb(args)?;
    let situation_args = args_without_flag_value(args, "--kb");
    if situation_args.is_empty() {
        return emit(ctx, render_usage_line("run requires <situation>"));
    }

    let situation = situation_args.join(" ");
    let tui_tx = ctx.tui_tx.clone();
    let llm = ctx.llm_adapter.clone();
    let kb_note = kb_pack_id
        .as_deref()
        .map(|kb| format!(" using KB `{kb}`"))
        .unwrap_or_default();
    emit(
        ctx,
        format!("Starting game-theory run{kb_note} for: {situation}\n"),
    )?;

    tokio::spawn(async move {
        let result = async {
            let db = open_db()?;
            let llm_ref = llm.as_ref().map(|arc| arc.as_ref() as &dyn LlmClient);
            gametheory::run_full_pipeline_with_options(
                &db,
                &situation,
                None,
                llm_ref,
                gametheory::GameTheoryMemoryContext::default(),
                gametheory::GameTheoryRunOptions {
                    kb_pack_id,
                    ..gametheory::GameTheoryRunOptions::default()
                },
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
        let _ = tui_tx.send(TuiEvent::TextDelta(msg));
    });
    Ok(())
}

fn start_classify_only(ctx: &mut CommandContext, args: &[String]) -> Result<()> {
    if args.is_empty() {
        return emit(ctx, render_usage_line("classify-only requires <situation>"));
    }

    let situation = args.join(" ");
    let tui_tx = ctx.tui_tx.clone();
    let llm = ctx.llm_adapter.clone();
    emit(
        ctx,
        format!("Classifying game-theory situation: {situation}\n"),
    )?;

    tokio::spawn(async move {
        let result = async {
            let db = open_db()?;
            let llm_ref = llm.as_ref().map(|arc| arc.as_ref() as &dyn LlmClient);
            let fingerprint = gametheory::classify(&db, &situation, llm_ref)
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
        let _ = tui_tx.send(TuiEvent::TextDelta(msg));
    });
    Ok(())
}

fn start_replay(ctx: &mut CommandContext, args: &[String]) -> Result<()> {
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
    emit(ctx, format!("Starting game-theory replay for {run_id}\n"))?;

    tokio::spawn(async move {
        let result = async {
            let db = open_db()?;
            if reclassify {
                let Some(situation) = gametheory_inspect::load_run_situation(&db, &run_id)? else {
                    anyhow::bail!("run not found: {run_id}");
                };
                let llm_ref = llm.as_ref().map(|arc| arc.as_ref() as &dyn LlmClient);
                let result = gametheory::run_full_pipeline_with_options(
                    &db,
                    &situation,
                    None,
                    llm_ref,
                    gametheory::GameTheoryMemoryContext::default(),
                    gametheory::GameTheoryRunOptions::default(),
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
            let result = gametheory::replay_single_specialist(
                &db,
                &run_id,
                &agent_key,
                llm_ref,
                gametheory::GameTheoryMemoryContext::default(),
                gametheory::GameTheoryRunOptions::default(),
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
        let _ = tui_tx.send(TuiEvent::TextDelta(msg));
    });
    Ok(())
}

fn render_specimens(db: &DbInstance, args: &[String]) -> Result<String> {
    let filter = parse_filter(args);
    let ingest = args.iter().any(|arg| arg == "--ingest");
    let load = gametheory::specimens::ensure_specimen_library_loaded(db, ingest)?;
    let rows = gametheory::specimens::list_specimens(db, filter.as_deref())?;

    let mut out = String::from("Game-Theory Specimens\n=====================\n");
    out.push_str(&format!(
        "Rows: {}\nInserted: {}\n",
        rows.len(),
        load.inserted
    ));
    for row in rows {
        out.push_str(&format!(
            "  {} cooperation={} payoff_sum={} timing={} horizon={}\n",
            row.situation_type, row.cooperation, row.payoff_sum, row.timing, row.horizon
        ));
    }
    Ok(out)
}

fn emit_db<F>(ctx: &mut CommandContext, render: F) -> Result<()>
where
    F: FnOnce(&DbInstance) -> Result<String>,
{
    let rendered = match ctx.cozo_db.as_ref() {
        Some(db) => render(db.as_ref())?,
        None => {
            let db = open_db()?;
            render(&db)?
        }
    };
    emit(ctx, rendered)
}

fn emit_db_event<F>(ctx: &mut CommandContext, render: F) -> Result<()>
where
    F: FnOnce(&DbInstance) -> Result<TuiEvent>,
{
    let event = match ctx.cozo_db.as_ref() {
        Some(db) => render(db.as_ref())?,
        None => {
            let db = open_db()?;
            render(&db)?
        }
    };
    ctx.emit(event);
    Ok(())
}

fn open_gametheory_rows_event(db: &DbInstance) -> Result<TuiEvent> {
    gametheory::schema::ensure_gametheory_schema(db)?;
    let rows = db
        .run_script(
            "?[run_id, situation, started_at, status, cost] := \
             *gt_runs{run_id, situation, started_at, completed_at, status, cost_usd: cost}",
            Default::default(),
            ScriptMutability::Immutable,
        )
        .map_err(|e| anyhow::anyhow!("query gt_runs for TUI view failed: {e}"))?;

    let rows = rows
        .rows
        .iter()
        .map(|row| EvidenceRowPayload {
            id: row[0].get_str().unwrap_or("").to_string(),
            title: row[1].get_str().unwrap_or("").to_string(),
            status: row[3].get_str().unwrap_or("").to_string(),
            detail: format!(
                "{} ${}",
                row[2].get_str().unwrap_or(""),
                row[4].get_str().unwrap_or("0.0")
            ),
        })
        .collect();
    Ok(TuiEvent::OpenViewRows {
        view_id: ViewId::GameTheory,
        rows,
    })
}

fn emit(ctx: &mut CommandContext, msg: String) -> Result<()> {
    ctx.emit(TuiEvent::TextDelta(msg));
    Ok(())
}

fn parse_tier(args: &[String]) -> Result<Option<u8>> {
    let Some(index) = args.iter().position(|arg| arg == "--tier") else {
        return Ok(None);
    };
    let Some(value) = args.get(index + 1) else {
        anyhow::bail!("--tier requires a numeric value");
    };
    Ok(Some(value.parse()?))
}

fn parse_filter(args: &[String]) -> Option<String> {
    for (idx, arg) in args.iter().enumerate() {
        if arg == "--filter" {
            return args.get(idx + 1).cloned();
        }
        if let Some(value) = arg.strip_prefix("--filter=") {
            return Some(value.to_string());
        }
    }
    None
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

fn render_usage() -> String {
    format!(
        "/gametheory subcommands: {}\n\nUsage:\n  /gametheory run <situation> [--kb <pack>]\n  /gametheory classify-only <situation>\n  /gametheory status [run-id]\n  /gametheory inspect <artifact-id>\n  /gametheory inspect-fingerprint <run-id>\n  /gametheory inspect-routing <run-id>\n  /gametheory list-runs\n  /gametheory show <run-id>\n  /gametheory replay <run-id> [--reclassify] [--rerun-specialist <key>]\n  /gametheory list-agents [--tier N]\n  /gametheory specimens [--filter axis=value] [--ingest]\n  /gametheory view\n",
        GAMETHEORY_SUBCOMMANDS.join(", ")
    )
}

fn render_usage_line(reason: &str) -> String {
    format!("{reason}\n\n{}", render_usage())
}

fn open_db() -> Result<DbInstance> {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from(".local/share"))
        .join("archon");
    std::fs::create_dir_all(&data_dir)?;
    let path = data_dir.join("archon-data.db");
    let path_str = path.to_string_lossy().to_string();
    DbInstance::new("sqlite", &path_str, "")
        .map_err(|e| anyhow::anyhow!("failed to open gametheory store at {path_str}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::{CommandHandler, default_registry};
    use crate::command::test_support::{CtxBuilder, drain_tui_events};

    #[test]
    fn test_gametheory_slash_declares_required_subcommands() {
        assert_eq!(
            GAMETHEORY_SUBCOMMANDS,
            &[
                "run",
                "classify-only",
                "status",
                "inspect",
                "inspect-fingerprint",
                "inspect-routing",
                "list-runs",
                "show",
                "replay",
                "list-agents",
                "specimens",
                "view"
            ]
        );
    }

    #[test]
    fn test_default_registry_registers_gametheory_primary() {
        let registry = default_registry();
        assert!(registry.is_primary("gametheory"));
        let handler = registry.get("gametheory").unwrap();
        assert_eq!(
            handler.description(),
            "Run and inspect the game-theory evidence pipeline"
        );
    }

    #[test]
    fn test_gametheory_usage_lists_all_subcommands() {
        let (mut ctx, mut rx) = CtxBuilder::new().build();
        GameTheorySlashHandler.execute(&mut ctx, &[]).unwrap();
        let events = drain_tui_events(&mut rx);
        let text = match &events[0] {
            TuiEvent::TextDelta(text) => text,
            other => panic!("expected TextDelta, got {other:?}"),
        };
        for subcommand in GAMETHEORY_SUBCOMMANDS {
            assert!(text.contains(subcommand), "missing {subcommand}");
        }
    }

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

    #[test]
    fn test_gametheory_view_emits_open_view_event() {
        let db = std::sync::Arc::new(test_db());
        gametheory::schema::ensure_gametheory_schema(db.as_ref()).unwrap();
        seed_gt_run(
            &db,
            "gt-slash-view",
            "marketplace incentives",
            "completed",
            "0.420000",
        );
        let (mut ctx, mut rx) = CtxBuilder::new().with_cozo_db(db).build();
        GameTheorySlashHandler
            .execute(&mut ctx, &[String::from("view")])
            .unwrap();
        let events = drain_tui_events(&mut rx);
        let [TuiEvent::OpenViewRows { view_id, rows }] = events.as_slice() else {
            panic!("expected OpenViewRows, got {events:?}");
        };
        assert_eq!(*view_id, ViewId::GameTheory);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "gt-slash-view");
        assert_eq!(rows[0].status, "completed");
        assert!(rows[0].detail.contains("$0.420000"));
    }

    #[test]
    fn test_gametheory_list_agents_uses_real_registry() {
        let (mut ctx, mut rx) = CtxBuilder::new().build();
        let args = vec![
            "list-agents".to_string(),
            "--tier".to_string(),
            "2".to_string(),
        ];
        GameTheorySlashHandler.execute(&mut ctx, &args).unwrap();
        let events = drain_tui_events(&mut rx);
        let text = match &events[0] {
            TuiEvent::TextDelta(text) => text,
            other => panic!("expected TextDelta, got {other:?}"),
        };
        assert!(text.contains("Tier Filter: 2"));
        assert!(text.contains("nash-equilibrium-finder"));
    }

    #[test]
    fn test_gametheory_status_reads_cozo_source_of_truth() {
        let db = std::sync::Arc::new(test_db());
        gametheory::schema::ensure_gametheory_schema(db.as_ref()).unwrap();
        seed_gt_run(
            &db,
            "gt-slash",
            "slash source truth",
            "completed",
            "0.010000",
        );

        let (mut ctx, mut rx) = CtxBuilder::new().with_cozo_db(db).build();
        let args = vec!["status".to_string(), "gt-slash".to_string()];
        GameTheorySlashHandler.execute(&mut ctx, &args).unwrap();
        let events = drain_tui_events(&mut rx);
        let text = match &events[0] {
            TuiEvent::TextDelta(text) => text,
            other => panic!("expected TextDelta, got {other:?}"),
        };
        assert!(text.contains("Run ID:    gt-slash"));
        assert!(text.contains("Status:    completed"));
        assert!(text.contains("Cost USD:  $0.010000"));
    }

    fn test_db() -> DbInstance {
        let path = format!("/tmp/test-gt-slash-{}.db", uuid::Uuid::new_v4());
        DbInstance::new("sqlite", &path, "").unwrap()
    }

    fn seed_gt_run(db: &DbInstance, run_id: &str, situation: &str, status: &str, cost: &str) {
        let mut params = std::collections::BTreeMap::new();
        params.insert("rid".into(), cozo::DataValue::from(run_id));
        params.insert("sit".into(), cozo::DataValue::from(situation));
        params.insert("status".into(), cozo::DataValue::from(status));
        params.insert("cost".into(), cozo::DataValue::from(cost));
        db.run_script(
            "?[run_id, situation, started_at, completed_at, status, cost_usd] \
             <- [[$rid, $sit, \"2026-05-03T00:00:00Z\", \
             \"2026-05-03T00:00:01Z\", $status, $cost]] \
             :put gt_runs { run_id => situation, started_at, completed_at, status, cost_usd }",
            params,
            ScriptMutability::Mutable,
        )
        .unwrap();
    }
}
