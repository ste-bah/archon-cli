//! Slash-to-CLI mirror handlers.
//!
//! These handlers give the TUI parity with the OS command line without
//! duplicating every CLI subcommand implementation. A slash command such as
//! `/kb claims` runs the same binary path as `archon kb claims` and emits the
//! captured stdout/stderr back into the TUI.

use anyhow::Result;
use archon_tui::app::TuiEvent;

use crate::command::registry::{CommandContext, CommandHandler};

pub(crate) struct CliMirrorHandler {
    prefix: Option<&'static str>,
    description: &'static str,
}

impl CliMirrorHandler {
    pub(crate) const fn archon() -> Self {
        Self {
            prefix: None,
            description: "Run any archon CLI command from inside the TUI",
        }
    }

    pub(crate) const fn prefixed(prefix: &'static str, description: &'static str) -> Self {
        Self {
            prefix: Some(prefix),
            description,
        }
    }
}

impl CommandHandler for CliMirrorHandler {
    fn execute(&self, ctx: &mut CommandContext, args: &[String]) -> Result<()> {
        let cli_args = mirror_args(self.prefix, args);
        if cli_args.is_empty() {
            return emit_usage(ctx, self.prefix);
        }

        let label = format!("archon {}", cli_args.join(" "));
        ctx.emit(TuiEvent::TextDelta(format!("Running `{label}`...\n")));
        let tui_tx = ctx.tui_tx.clone();
        let task_name = format!("cli-mirror:{label}");

        archon_observability::spawn_named(task_name, async move {
            let _workload_guard = archon_tui::observability::LongRunningWorkloadGuard::new(&label);
            let rendered = match run_archon(cli_args).await {
                Ok(outcome) => outcome.render(&label),
                Err(err) => format!("`{label}` failed to launch: {err}\n"),
            };
            let _ = tui_tx.send_async(TuiEvent::TextDelta(rendered)).await;
        });
        Ok(())
    }

    fn description(&self) -> &str {
        self.description
    }
}

pub(crate) fn spawn_cli_mirror(
    ctx: &mut CommandContext,
    prefix: &'static str,
    args: &[String],
) -> Result<()> {
    CliMirrorHandler::prefixed(prefix, "Run a mirrored archon CLI command").execute(ctx, args)
}

fn emit_usage(ctx: &mut CommandContext, prefix: Option<&str>) -> Result<()> {
    let usage = match prefix {
        Some(prefix) => format!("Usage: /{prefix} <subcommand> [args]\nMirrors `archon {prefix} ...` inside the TUI.\n"),
        None => "Usage: /archon <cli-subcommand> [args]\nExample: /archon docs ingest .archon/docs/inbox\n".to_string(),
    };
    ctx.emit(TuiEvent::TextDelta(usage));
    Ok(())
}

fn mirror_args(prefix: Option<&str>, args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(prefix) = prefix {
        out.push(prefix.to_string());
    }
    out.extend(args.iter().cloned());
    out
}

/// What a mirrored CLI run produced: the child's exit status alongside its
/// captured streams.
///
/// The status is carried rather than folded into the text because the TUI has
/// no exit code of its own. Commands that mean something *by* exiting non-zero
/// — `archon cognitive gate` is the one that forced this — would otherwise be
/// announced with the same "completed" line as a clean run, and a gate that
/// reads as a pass when it failed is worse than no gate in the TUI at all.
struct MirrorOutcome {
    status: std::process::ExitStatus,
    streams: String,
}

impl MirrorOutcome {
    fn render(&self, label: &str) -> String {
        let streams = if self.streams.trim().is_empty() {
            "(no output)\n"
        } else {
            &self.streams
        };
        if self.status.success() {
            format!("`{label}` completed\n\n{streams}")
        } else {
            format!(
                "`{label}` FAILED — {}\n\n{streams}",
                describe_status(self.status)
            )
        }
    }
}

/// `ExitStatus`'s own `Display` reads "exit code: 1" on Windows and "exit status: 1"
/// elsewhere; the prefix is normalised so the failure line does not depend on
/// which OS the TUI is running on.
fn describe_status(status: std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit status {code}"),
        None => format!("terminated by signal ({status})"),
    }
}

async fn run_archon(args: Vec<String>) -> Result<MirrorOutcome> {
    let exe = std::env::current_exe()?;
    let output = tokio::process::Command::new(exe)
        .args(args)
        .output()
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut streams = String::new();
    if !stdout.trim().is_empty() {
        streams.push_str(stdout.trim_end());
        streams.push('\n');
    }
    if !stderr.trim().is_empty() {
        streams.push_str("\nstderr:\n");
        streams.push_str(stderr.trim_end());
        streams.push('\n');
    }
    Ok(MirrorOutcome {
        status: output.status,
        streams,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::default_registry;

    fn exit_status(code: i32) -> std::process::ExitStatus {
        #[cfg(windows)]
        {
            use std::os::windows::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(code as u32)
        }
        #[cfg(not(windows))]
        {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(code << 8)
        }
    }

    /// A mirrored command that exits non-zero must say so before anything the
    /// command itself printed. `/cognitive gate` reports "Verdict: FAIL" on
    /// stdout and bails to stderr, but both scroll; the first line does not.
    #[test]
    fn non_zero_exit_is_announced_before_the_output() {
        let outcome = MirrorOutcome {
            status: exit_status(1),
            streams: "Verdict: FAIL\n".to_string(),
        };
        let rendered = outcome.render("archon cognitive gate");
        let first_line = rendered.lines().next().expect("rendered output has a line");
        assert!(
            first_line.contains("FAILED") && first_line.contains("exit status 1"),
            "first line must name the failure, got {first_line:?}"
        );
        assert!(
            !first_line.contains("completed"),
            "a failed run must not be announced as completed, got {first_line:?}"
        );
        assert!(rendered.contains("Verdict: FAIL"));
    }

    #[test]
    fn zero_exit_keeps_the_completed_wording() {
        let outcome = MirrorOutcome {
            status: exit_status(0),
            streams: "Verdict: pass\n".to_string(),
        };
        let rendered = outcome.render("archon cognitive gate");
        assert!(rendered.starts_with("`archon cognitive gate` completed"));
        assert!(!rendered.contains("FAILED"));
    }

    #[test]
    fn silent_run_still_reports_something() {
        let outcome = MirrorOutcome {
            status: exit_status(0),
            streams: String::new(),
        };
        assert!(outcome.render("archon kb claims").contains("(no output)"));
    }

    #[test]
    fn mirror_args_prefixes_family_command() {
        let args = vec!["claims".to_string()];
        assert_eq!(mirror_args(Some("kb"), &args), vec!["kb", "claims"]);
    }

    #[test]
    fn mirror_args_archon_passthrough_preserves_cli_shape() {
        let args = vec!["docs".to_string(), "status".to_string()];
        assert_eq!(mirror_args(None, &args), vec!["docs", "status"]);
    }

    #[test]
    fn registry_exposes_cli_mirror_primaries() {
        let registry = default_registry();
        for primary in [
            "archon",
            "kb",
            "prov",
            "meaning",
            "constellation",
            "completion",
            "behaviour",
            "pipeline",
            "reasoning",
            "briefing",
            "auth",
            "chat",
            "trading",
        ] {
            assert!(
                registry.is_primary(primary),
                "/{primary} must be registered"
            );
        }
    }
}
