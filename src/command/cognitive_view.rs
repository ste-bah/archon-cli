//! Slash handler for the cognitive executive-state TUI inspection view.

use anyhow::Result;
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};

use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) struct CognitiveViewHandler;

impl CommandHandler for CognitiveViewHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        match args.first().map(String::as_str) {
            None | Some("open" | "view") => open_view(ctx),
            Some("status" | "tick" | "inspect" | "self-model" | "reflections") => {
                crate::command::cli_mirror::spawn_cli_mirror(ctx, "cognitive", args)?;
            }
            Some("help") => emit(ctx, usage())?,
            Some(other) => emit(
                ctx,
                format!("unknown cognitive subcommand `{other}`\n\n{}", usage()),
            )?,
        }
        Ok(())
    }

    fn description(&self) -> &str {
        "Open the cognitive executive-state TUI browser"
    }
}

fn open_view(ctx: &mut CommandContext) {
    let cwd = ctx
        .working_dir
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let root = cwd.join(".archon").join("cognitive");
    let event = match archon_cognitive::PersistentCognitiveStore::open(&root).and_then(|store| {
        let inspection = archon_cognitive::CognitiveInspection::new(store.db(), &root)?;
        inspection.status()
    }) {
        Ok(status) => TuiEvent::OpenViewRows {
            view_id: ViewId::Cognitive,
            rows: rows(status),
        },
        Err(err) => TuiEvent::TextDelta(format!("cognitive inspection unavailable: {err}")),
    };
    ctx.emit(event);
}

fn rows(status: archon_cognitive::CognitiveInspectionStatus) -> Vec<EvidenceRowPayload> {
    let mut rows = vec![
        summary_row(
            "situations",
            status.situation_count,
            "classified turn records",
        ),
        summary_row(
            "tool_decisions",
            status.tool_decision_count,
            "tool gate outcomes",
        ),
        summary_row(
            "reflections",
            status.reflection_count,
            "safe lessons without raw turn text",
        ),
        EvidenceRowPayload {
            id: "self_model".into(),
            title: "self model".into(),
            status: status.self_model_fact_count.to_string(),
            detail: format!("{} caution rule(s)", status.self_model.caution_rules.len()),
        },
    ];
    rows.extend(
        status
            .recent_decisions
            .into_iter()
            .map(|decision| EvidenceRowPayload {
                id: decision.decision_id,
                title: "decision".into(),
                status: decision.selected_candidate_id,
                detail: decision.user_visible_summary,
            }),
    );
    rows.extend(
        status
            .recent_reflections
            .into_iter()
            .map(|reflection| EvidenceRowPayload {
                id: reflection.reflection_id,
                title: "reflection".into(),
                status: reflection.outcome,
                detail: reflection.lesson,
            }),
    );
    rows
}

fn summary_row(id: &str, count: usize, detail: &str) -> EvidenceRowPayload {
    EvidenceRowPayload {
        id: id.into(),
        title: id.replace('_', " "),
        status: count.to_string(),
        detail: detail.into(),
    }
}

fn emit(ctx: &mut CommandContext, msg: String) -> Result<()> {
    ctx.emit(TuiEvent::TextDelta(msg));
    Ok(())
}

fn usage() -> String {
    "Usage: /cognitive [open|view|status|tick|daemon|inspect|self-model|reflections]\n\
     Opens the read-only executive-state browser or mirrors `archon cognitive ...`."
        .into()
}
