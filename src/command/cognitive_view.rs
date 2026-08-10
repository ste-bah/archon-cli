//! Slash handler for the cognitive executive-state TUI inspection view.

use anyhow::Result;
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};

use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) struct CognitiveViewHandler;

impl CommandHandler for CognitiveViewHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        match args.first().map(String::as_str) {
            None | Some("open" | "view") => open_view(ctx),
            Some(subcommand) if mirrors_cli(subcommand) => {
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

/// Which `archon cognitive ...` subcommands `/cognitive ...` forwards to the CLI.
///
/// `gate` is here despite reporting its verdict through an exit code, which a
/// slash command has no equivalent of: the mirror renders a non-zero exit as a
/// `FAILED` first line, so the TUI form is as loud as the shell form. Leaving it
/// out was an omission, not a decision — the gate is a read-only judgement over
/// the same store `/cognitive` already browses, and every other subcommand of
/// the family, including the mutating `tick` and `daemon start`, is mirrored.
fn mirrors_cli(subcommand: &str) -> bool {
    matches!(
        subcommand,
        "status"
            | "tick"
            | "gate"
            | "adjudicate"
            | "daemon"
            | "inspect"
            | "self-model"
            | "reflections"
    )
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
        summary_row(
            "metric_events",
            status.metric_event_count,
            "append-only measurement source of truth",
        ),
    ];
    rows.extend(metric_rows(&status.metrics));
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

/// One row per derived metric per cohort. The cohort is part of the row id so
/// a segment can be searched for directly instead of being hidden inside a
/// pooled aggregate.
fn metric_rows(snapshot: &archon_cognitive::CognitiveMetricSnapshot) -> Vec<EvidenceRowPayload> {
    snapshot
        .metrics
        .iter()
        .map(|metric| EvidenceRowPayload {
            id: format!(
                "{}@{}",
                metric.metric_name,
                metric.cohort.segmentation_key()
            ),
            title: "metric".into(),
            status: metric
                .value
                .map(|value| format!("{value:.4}"))
                .unwrap_or_else(|| "undefined".to_string()),
            detail: format!(
                "{} [{}] n={}",
                metric.metric_name,
                metric.cohort.segmentation_key(),
                metric.sample_count
            ),
        })
        .collect()
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
    "Usage: /cognitive [open|view|status|tick|gate|adjudicate|daemon|inspect|self-model|reflections]\n\
     Opens the read-only executive-state browser or mirrors `archon cognitive ...`.\n\
     `gate` fails loudly: a non-zero exit is reported as a FAILED run, not a completed one."
        .into()
}

#[cfg(test)]
mod tests {
    use super::mirrors_cli;

    #[test]
    fn daemon_subcommand_is_mirrored_to_cli() {
        assert!(mirrors_cli("daemon"));
    }

    /// `archon cognitive gate` shipped without a slash form (#83 added the CLI,
    /// `mirrors_cli` was never extended), so `/cognitive gate` answered "unknown
    /// cognitive subcommand" for a command that exists.
    #[test]
    fn gate_subcommand_is_mirrored_to_cli() {
        assert!(mirrors_cli("gate"));
    }

    /// #77 added `archon cognitive adjudicate` as the surface a human uses to
    /// settle a proposed causal attribution. Without this token the slash form
    /// answers "unknown cognitive subcommand", which reads as "the feature is
    /// missing" rather than "the mirror is short a word" — the same shape as
    /// `gate` above.
    #[test]
    fn adjudicate_subcommand_is_mirrored_to_cli() {
        assert!(mirrors_cli("adjudicate"));
    }

    #[test]
    fn usage_lists_every_mirrored_subcommand() {
        let usage = super::usage();
        for subcommand in [
            "status",
            "tick",
            "gate",
            "adjudicate",
            "daemon",
            "inspect",
            "self-model",
            "reflections",
        ] {
            assert!(
                mirrors_cli(subcommand),
                "{subcommand} is listed in usage but not mirrored"
            );
            assert!(
                usage.contains(subcommand),
                "{subcommand} is mirrored but missing from usage"
            );
        }
    }

    #[test]
    fn unknown_subcommand_is_not_mirrored() {
        assert!(!mirrors_cli("nope"));
    }
}
