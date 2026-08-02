//! Slash handler for the world-model TUI inspector and approval gate.
//!
//! Closes the `archon world ...` shell-only surface exception. Two halves,
//! matching what that exception deferred on:
//!
//!   1. **Inspector** — `/world` (or `/world status`) opens a read-only view
//!      of advisor state, corpus/cold-start progress, candidates, last eval,
//!      and daemon-trainer health.
//!   2. **Approval gate** — the verbs that change which model the advisory
//!      runs against, or that write to the trace corpus, require an explicit
//!      `--yes` when invoked from the TUI. Without it the command is not run;
//!      the handler explains what it would do and how to confirm.
//!
//! Read-only verbs pass straight through to the CLI mirror, so the TUI has
//! full parity with `archon world` without duplicating any implementation.
//!
//! The gate is deliberately a confirmation, not a permission check: it exists
//! so `promote` and `rollback` cannot be one keystroke away in an interactive
//! session, matching the `docs delete --yes` precedent. Shell invocations are
//! unaffected — `archon world promote` behaves exactly as before.

use anyhow::Result;
use archon_tui::app::{EvidenceRowPayload, TuiEvent, ViewId};

use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) struct WorldViewHandler;

/// Verbs that mutate promotion state, the trace corpus, or guardrail policy.
/// Everything else is inspection and passes through unguarded.
const GUARDED: &[&str] = &[
    "promote",
    "promote-jepa",
    "rollback",
    "guard",
    "train",
    "train-jepa",
    "trainer-tick",
    "ingest",
    "record-outcome",
    "eval-jepa-cancel",
];

const READ_ONLY: &[&str] = &[
    "status",
    "predict-next",
    "score-actions",
    "explain",
    "eval",
    "eval-jepa",
    "eval-jepa-status",
    "eval-jepa-runs",
    "inspect-jepa",
    "compare-representations",
];

impl CommandHandler for WorldViewHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        match args.first().map(String::as_str) {
            None | Some("open" | "view") => open_view(ctx),
            Some("help") => emit(ctx, usage()),
            Some(verb) if READ_ONLY.contains(&verb) => {
                crate::command::cli_mirror::spawn_cli_mirror(ctx, "world", args)?;
            }
            Some(verb) if GUARDED.contains(&verb) => {
                if approved(args) {
                    crate::command::cli_mirror::spawn_cli_mirror(ctx, "world", args)?;
                } else {
                    emit(ctx, confirmation_prompt(verb, args));
                }
            }
            Some(other) => emit(
                ctx,
                format!("unknown world subcommand `{other}`\n\n{}", usage()),
            ),
        }
        Ok(())
    }

    fn description(&self) -> &str {
        "Inspect the local world model; promotion and rollback require --yes"
    }
}

fn approved(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--yes")
}

fn open_view(ctx: &mut CommandContext) {
    let config = archon_core::config::load_config().unwrap_or_default();
    let rows = crate::command::world_model::world_inspection_rows(&config)
        .into_iter()
        .map(to_payload)
        .collect();
    ctx.emit(TuiEvent::OpenViewRows {
        view_id: ViewId::World,
        rows,
    });
}

fn to_payload(row: crate::command::world_model::WorldInspectionRow) -> EvidenceRowPayload {
    EvidenceRowPayload {
        id: row.id.to_string(),
        title: row.label,
        status: row.status,
        detail: row.detail,
    }
}

fn confirmation_prompt(verb: &str, args: &[String]) -> String {
    let rendered = args.join(" ");
    format!(
        "`/world {verb}` changes world-model state and needs explicit confirmation.\n\
         \n\
         {}\n\
         \n\
         Re-run with --yes to proceed:\n    /world {rendered} --yes\n\
         \n\
         Inspect first with `/world` (advisor, corpus, candidates, last eval).\n",
        effect_of(verb)
    )
}

fn effect_of(verb: &str) -> &'static str {
    match verb {
        "promote" | "promote-jepa" => {
            "This changes which model the live advisory runs against. Check `/world` shows a scored candidate before promoting."
        }
        "rollback" => "This reverts the active model to a previous promotion.",
        "guard" => "This changes runtime guardrail policy, which gates tool admission.",
        "train" | "train-jepa" => {
            "This starts a training run and writes a new candidate model."
        }
        "trainer-tick" => "This forces a daemon trainer tick outside its schedule.",
        "ingest" => "This writes to the trace corpus the model trains on.",
        "record-outcome" => "This writes an outcome label into the trace corpus.",
        "eval-jepa-cancel" => "This cancels an in-flight JEPA evaluation run.",
        _ => "This mutates world-model state.",
    }
}

fn emit(ctx: &mut CommandContext, text: String) {
    ctx.emit(TuiEvent::TextDelta(text));
}

fn usage() -> String {
    format!(
        "/world — local world-model inspector\n\
         \n\
         /world                  open the inspector (advisor, corpus, candidates, eval, trainer)\n\
         /world help             this message\n\
         \n\
         Read-only (run directly):\n    {}\n\
         \n\
         Requires --yes (changes model or corpus state):\n    {}\n",
        READ_ONLY.join(", "),
        GUARDED.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarded_and_read_only_verbs_are_disjoint() {
        for verb in GUARDED {
            assert!(
                !READ_ONLY.contains(verb),
                "{verb} is classified as both guarded and read-only"
            );
        }
    }

    #[test]
    fn promotion_and_rollback_are_guarded() {
        for verb in ["promote", "promote-jepa", "rollback", "guard"] {
            assert!(GUARDED.contains(&verb), "{verb} must require confirmation");
        }
    }

    #[test]
    fn approval_requires_explicit_yes_flag() {
        assert!(!approved(&["promote".to_string()]));
        assert!(!approved(&["promote".to_string(), "--dry-run".to_string()]));
        assert!(approved(&["promote".to_string(), "--yes".to_string()]));
    }

    #[test]
    fn confirmation_prompt_names_the_effect_and_the_flag() {
        let prompt = confirmation_prompt("promote", &["promote".to_string()]);
        assert!(prompt.contains("--yes"));
        assert!(prompt.contains("live advisory"));
    }

    #[test]
    fn usage_lists_every_verb() {
        let usage = usage();
        for verb in GUARDED.iter().chain(READ_ONLY.iter()) {
            assert!(usage.contains(*verb), "usage omits {verb}");
        }
    }
}
