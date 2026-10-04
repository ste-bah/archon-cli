//! `/gametheory` slash-command umbrella.

use anyhow::Result;
use archon_pipeline::gametheory;
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};
use cozo::{DbInstance, ScriptMutability};

use crate::command::gametheory_inspect;
use crate::command::registry::{CommandContext, CommandHandler};

#[path = "gametheory_slash_execution.rs"]
mod execution;
use execution::{start_classify_only, start_replay, start_run};

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
    // Gametheory slash writers open fresh DbInstance handles inside spawned
    // tasks. Mirror that on reads so same-session inspect commands see rows
    // committed by sibling Cozo/SQLite connections.
    let db = open_db()?;
    let rendered = render(&db)?;
    emit(ctx, rendered)
}

fn emit_db_event<F>(ctx: &mut CommandContext, render: F) -> Result<()>
where
    F: FnOnce(&DbInstance) -> Result<TuiEvent>,
{
    // Same fresh-read rule as emit_db; ctx.cozo_db can be a stale session
    // snapshot for gametheory rows written by background slash tasks.
    let db = open_db()?;
    let event = render(&db)?;
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

fn render_usage() -> String {
    format!(
        "/gametheory subcommands: {}\n\nUsage:\n  /gametheory run <situation> [--kb <pack>]\n  /gametheory classify-only <situation>\n  /gametheory status [run-id]\n  /gametheory inspect <artifact-id>\n  /gametheory inspect-fingerprint <run-id>\n  /gametheory inspect-routing <run-id>\n  /gametheory list-runs\n  /gametheory show <run-id>\n  /gametheory replay <run-id> [--reclassify] [--rerun-specialist <key>]\n  /gametheory list-agents [--tier N]\n  /gametheory specimens [--filter axis=value] [--ingest]\n  /gametheory view\n",
        GAMETHEORY_SUBCOMMANDS.join(", ")
    )
}

fn render_usage_line(reason: &str) -> String {
    format!("{reason}\n\n{}", render_usage())
}

fn open_db() -> Result<std::sync::Arc<DbInstance>> {
    crate::command::store_paths::open_evidence_db("gametheory", &["ARCHON_GAMETHEORY_DB_PATH"])
}

#[cfg(test)]
#[path = "gametheory_slash_tests.rs"]
mod tests;
