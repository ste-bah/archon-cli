//! Code-owned CLI/slash/TUI parity matrix.
//!
//! The docs say "all CLI surfaces should be reachable from the TUI";
//! this module turns that into a testable table instead of prose.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceStatus {
    Done,
    Partial,
    ShellOnly,
}

impl SurfaceStatus {
    fn as_str(self) -> &'static str {
        match self {
            SurfaceStatus::Done => "DONE",
            SurfaceStatus::Partial => "PARTIAL",
            SurfaceStatus::ShellOnly => "SHELL_ONLY",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CommandSurfaceRow {
    pub(crate) cli: &'static str,
    pub(crate) slash_primary: Option<&'static str>,
    pub(crate) tui_surface: &'static str,
    pub(crate) status: SurfaceStatus,
    pub(crate) source_of_truth: &'static str,
    pub(crate) notes: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SurfaceException {
    pub(crate) cli: &'static str,
    pub(crate) owner: &'static str,
    pub(crate) review_date: &'static str,
    pub(crate) reason: &'static str,
}

pub(crate) const COMMAND_SURFACE_ROWS: &[CommandSurfaceRow] = &[
    CommandSurfaceRow {
        cli: "archon auth ...",
        slash_primary: Some("auth"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/registry.rs + src/cli_args.rs",
        notes: "Provider login/status/logout is available from shell and TUI.",
    },
    CommandSurfaceRow {
        cli: "archon chat --provider <id> <prompt>",
        slash_primary: Some("chat"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/registry.rs + src/command/chat.rs",
        notes: "One-shot provider chat is mirrored into the TUI.",
    },
    CommandSurfaceRow {
        cli: "archon providers ...",
        slash_primary: Some("providers"),
        tui_surface: "Direct slash handler",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/providers.rs",
        notes: "Provider list, capabilities, and doctor are available in both surfaces.",
    },
    CommandSurfaceRow {
        cli: "archon docs ...",
        slash_primary: Some("docs"),
        tui_surface: "Evidence browser + CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/docs.rs + src/command/evidence_view.rs",
        notes: "Document ingest/search/inspect routes through persisted document state.",
    },
    CommandSurfaceRow {
        cli: "archon kb ...",
        slash_primary: Some("kb"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/registry.rs",
        notes: "Knowledge claims, entities, relations, contradictions, and search are mirrored.",
    },
    CommandSurfaceRow {
        cli: "archon prov ...",
        slash_primary: Some("prov"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/registry.rs",
        notes: "Trace, export, and verify run through the same provenance store.",
    },
    CommandSurfaceRow {
        cli: "archon gametheory ...",
        slash_primary: Some("gametheory"),
        tui_surface: "Direct slash handler",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/gametheory_slash.rs",
        notes: "Run, classify-only, status, inspect, replay, agents, and specimens are exposed.",
    },
    CommandSurfaceRow {
        cli: "archon completion ...",
        slash_primary: Some("completion"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/registry.rs",
        notes: "Completion integrity inspection and trust surfaces are mirrored.",
    },
    CommandSurfaceRow {
        cli: "archon behaviour ...",
        slash_primary: Some("behaviour"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/registry.rs",
        notes: "Governed-learning events, proposals, approvals, rollback, and status are mirrored.",
    },
    CommandSurfaceRow {
        cli: "archon reasoning ...",
        slash_primary: Some("reasoning"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/reasoning.rs + crates/archon-reasoning-quality",
        notes: "Reasoning-quality status, inspection, claims, patterns, backfill, fixture audit, shadow report, migrations, and dead-letter replay are mirrored.",
    },
    CommandSurfaceRow {
        cli: "archon briefing ...",
        slash_primary: Some("briefing"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/reasoning.rs + src/runtime/proactive_briefing.rs",
        notes: "Proactive session briefing preview is mirrored for TUI validation.",
    },
    CommandSurfaceRow {
        cli: "archon meaning ...",
        slash_primary: Some("meaning"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/registry.rs",
        notes: "Meaning samples, contrastive pairs, triplets, and export are mirrored.",
    },
    CommandSurfaceRow {
        cli: "archon constellation ...",
        slash_primary: Some("constellation"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/registry.rs",
        notes: "Centroid build, bootstrap, score, drift, and list commands are mirrored.",
    },
    CommandSurfaceRow {
        cli: "archon learning ...",
        slash_primary: Some("learning"),
        tui_surface: "Direct slash handler",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/evidence_view.rs",
        notes: "Learning view plus GNN auto-trainer status diagnostics.",
    },
    CommandSurfaceRow {
        cli: "archon pipeline ...",
        slash_primary: Some("pipeline"),
        tui_surface: "CLI mirror",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/registry.rs",
        notes: "Pipeline run/status/resume/list/abort/cancel plus audited verify/inspect/export are mirrored.",
    },
    CommandSurfaceRow {
        cli: "archon pipeline code <task>",
        slash_primary: Some("archon-code"),
        tui_surface: "Pipeline primary",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/archon_code.rs",
        notes: "The coding pipeline has a first-class TUI slash primary; continuation uses /pipeline resume <session-id>.",
    },
    CommandSurfaceRow {
        cli: "archon pipeline research <topic>",
        slash_primary: Some("archon-research"),
        tui_surface: "Pipeline primary",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/archon_research.rs",
        notes: "The research pipeline has a first-class TUI slash primary; continuation uses /pipeline resume <session-id>.",
    },
    CommandSurfaceRow {
        cli: "archon agent-list/search/info",
        slash_primary: Some("agent"),
        tui_surface: "Agent umbrella",
        status: SurfaceStatus::Done,
        source_of_truth: "src/command/agent_slash.rs",
        notes: "Agent list, info, and run are grouped under /agent.",
    },
    CommandSurfaceRow {
        cli: "archon run-agent-async ...",
        slash_primary: Some("run-agent"),
        tui_surface: "Custom-agent launcher",
        status: SurfaceStatus::Partial,
        source_of_truth: "src/command/run_agent.rs + src/command/task.rs",
        notes: "Launch is slash-native; async task status/result/cancel/list/events use /tasks and shell commands.",
    },
    CommandSurfaceRow {
        cli: "archon task-status/result/cancel/list/events",
        slash_primary: Some("tasks"),
        tui_surface: "Task browser",
        status: SurfaceStatus::Partial,
        source_of_truth: "src/command/task.rs",
        notes: "/tasks covers listing and task visibility; individual shell subcommands remain richer.",
    },
    CommandSurfaceRow {
        cli: "archon plugin ...",
        slash_primary: Some("plugin"),
        tui_surface: "Plugin umbrella",
        status: SurfaceStatus::Partial,
        source_of_truth: "src/command/plugin_slash.rs",
        notes: "List/info are live; enable/disable/install/reload emit guidance until persistent plugin state exists.",
    },
    CommandSurfaceRow {
        cli: "archon self ...",
        slash_primary: None,
        tui_surface: "Calibration shell",
        status: SurfaceStatus::ShellOnly,
        source_of_truth: "src/cli_args.rs + src/command/self_calibration.rs",
        notes: "Hybrid retrospective analysis, self-trust, and plan-vs-outcome inspection are shell-first calibration tools.",
    },
    CommandSurfaceRow {
        cli: "archon world ...",
        slash_primary: None,
        tui_surface: "World-model shell",
        status: SurfaceStatus::ShellOnly,
        source_of_truth: "src/cli_args.rs + src/command/world_model.rs + crates/archon-world-model",
        notes: "Local world-model status, ingest/backfill, dynamic trainer tick, latent and JEPA candidate train/eval/promote, JEPA eval-run inspection, fail-open prediction, outcome/surprise recording, action scoring, explain, and rollback are shell-first while the advisor remains advisory-only.",
    },
    CommandSurfaceRow {
        cli: "archon team ...",
        slash_primary: None,
        tui_surface: "Not yet mirrored",
        status: SurfaceStatus::ShellOnly,
        source_of_truth: "src/cli_args.rs + src/command/team.rs",
        notes: "Team execution is shell-only until a /team handler is wired.",
    },
    CommandSurfaceRow {
        cli: "archon serve/remote/web/ide-stdio",
        slash_primary: None,
        tui_surface: "Host process control",
        status: SurfaceStatus::ShellOnly,
        source_of_truth: "src/cli_args.rs",
        notes: "Process-mode commands intentionally remain shell-only.",
    },
    CommandSurfaceRow {
        cli: "archon metrics/update",
        slash_primary: None,
        tui_surface: "Operations shell",
        status: SurfaceStatus::ShellOnly,
        source_of_truth: "src/cli_args.rs",
        notes: "Operational commands are shell-first; TUI mirrors can be added if product need appears.",
    },
];

pub(crate) const COMMAND_SURFACE_EXCEPTIONS: &[SurfaceException] = &[
    SurfaceException {
        cli: "archon run-agent-async ...",
        owner: "archon-maintainers",
        review_date: "2026-06-30",
        reason: "Launch is TUI-native; richer async task verbs remain under `/tasks` until the task detail screen lands.",
    },
    SurfaceException {
        cli: "archon task-status/result/cancel/list/events",
        owner: "archon-maintainers",
        review_date: "2026-06-30",
        reason: "`/tasks` is the approved TUI entry point; per-id shell verbs stay richer until task drill-down UX is built.",
    },
    SurfaceException {
        cli: "archon plugin ...",
        owner: "archon-maintainers",
        review_date: "2026-06-30",
        reason: "List/info are live; mutating plugin operations remain guided until persistent plugin state is productized.",
    },
    SurfaceException {
        cli: "archon self ...",
        owner: "archon-maintainers",
        review_date: "2026-09-30",
        reason: "Self-calibration is intentionally shell-first until retrospective review has a dedicated TUI inspector.",
    },
    SurfaceException {
        cli: "archon world ...",
        owner: "archon-maintainers",
        review_date: "2026-09-30",
        reason: "World-model controls are intentionally shell-first until a dedicated TUI inspector and approval workflow lands.",
    },
    SurfaceException {
        cli: "archon team ...",
        owner: "archon-maintainers",
        review_date: "2026-06-30",
        reason: "Team execution is intentionally shell-only until a first-class team command-center workflow is designed.",
    },
    SurfaceException {
        cli: "archon serve/remote/web/ide-stdio",
        owner: "archon-maintainers",
        review_date: "2026-06-30",
        reason: "Host process control is intentionally shell-only and not part of the interactive command center.",
    },
    SurfaceException {
        cli: "archon metrics/update",
        owner: "archon-maintainers",
        review_date: "2026-06-30",
        reason: "Operational maintenance commands are approved shell-only surfaces unless a product need appears.",
    },
];

pub(crate) fn command_surface_rows() -> &'static [CommandSurfaceRow] {
    COMMAND_SURFACE_ROWS
}

pub(crate) fn command_surface_exception(cli: &str) -> Option<&'static SurfaceException> {
    COMMAND_SURFACE_EXCEPTIONS
        .iter()
        .find(|exception| exception.cli == cli)
}

pub(crate) fn render_command_surface_markdown() -> String {
    let mut out = String::new();
    out.push_str("# Command surface matrix\n\n");
    out.push_str("Generated from `src/command/surface_matrix.rs`. ");
    out.push_str(
        "Update the code-owned matrix and regenerate this file when command surfaces change.\n\n",
    );
    out.push_str(
        "Rows marked `PARTIAL` or `SHELL_ONLY` must carry an approved exception with an owner and review date.\n\n",
    );
    out.push_str(
        "| CLI surface | Slash primary | TUI surface | Status | Source of truth | Notes | Approved exception |\n",
    );
    out.push_str("|---|---|---|---|---|---|---|\n");
    for row in command_surface_rows() {
        let slash = row
            .slash_primary
            .map(|primary| format!("`/{primary}`"))
            .unwrap_or_else(|| "-".to_string());
        let exception = command_surface_exception(row.cli)
            .map(|exception| {
                format!(
                    "{}; owner: {}; review: {}",
                    exception.reason, exception.owner, exception.review_date
                )
            })
            .unwrap_or_else(|| "-".to_string());
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | `{}` | {} | {} |\n",
            row.cli,
            slash,
            row.tui_surface,
            row.status.as_str(),
            row.source_of_truth,
            row.notes,
            exception
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::command::registry::default_registry;

    #[test]
    fn generated_command_surface_doc_matches_code() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/generated/command-surface-matrix.md");

        let generated = fs::read_to_string(path).expect("generated command matrix doc exists");
        assert_eq!(generated, render_command_surface_markdown());
    }

    #[test]
    fn slash_rows_are_registered_primaries() {
        let registry = default_registry();

        for row in command_surface_rows() {
            if let Some(primary) = row.slash_primary {
                assert!(
                    registry.is_primary(primary),
                    "{} maps to /{primary}, but that primary is not registered",
                    row.cli
                );
            }
        }
    }

    #[test]
    fn required_prd_command_families_have_tui_entries() {
        let required = [
            "archon docs ...",
            "archon kb ...",
            "archon prov ...",
            "archon gametheory ...",
            "archon completion ...",
            "archon behaviour ...",
            "archon meaning ...",
            "archon constellation ...",
            "archon auth ...",
            "archon chat --provider <id> <prompt>",
            "archon providers ...",
        ];

        let rows: HashSet<_> = command_surface_rows().iter().map(|row| row.cli).collect();
        for cli in required {
            assert!(
                rows.contains(cli),
                "{cli} is missing from the surface matrix"
            );
        }
    }

    #[test]
    fn shell_only_rows_do_not_claim_slash_support() {
        for row in command_surface_rows() {
            if row.status == SurfaceStatus::ShellOnly {
                assert!(
                    row.slash_primary.is_none(),
                    "{} is shell-only but claims slash support",
                    row.cli
                );
            }
        }
    }

    #[test]
    fn non_done_rows_have_approved_exceptions() {
        for row in command_surface_rows() {
            if row.status != SurfaceStatus::Done {
                let exception = command_surface_exception(row.cli)
                    .unwrap_or_else(|| panic!("{} has no approved exception", row.cli));
                assert!(
                    !exception.owner.is_empty(),
                    "{} exception has no owner",
                    row.cli
                );
                assert!(
                    !exception.review_date.is_empty(),
                    "{} exception has no review date",
                    row.cli
                );
                assert!(
                    !exception.reason.is_empty(),
                    "{} exception has no reason",
                    row.cli
                );
            }
        }
    }

    #[test]
    fn exception_rows_match_real_command_rows() {
        let rows: HashSet<_> = command_surface_rows().iter().map(|row| row.cli).collect();
        for exception in COMMAND_SURFACE_EXCEPTIONS {
            assert!(
                rows.contains(exception.cli),
                "exception references missing CLI surface {}",
                exception.cli
            );
        }
    }
}
