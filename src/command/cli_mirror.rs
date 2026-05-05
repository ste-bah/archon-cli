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
            let output = run_archon(cli_args).await;
            let rendered = match output {
                Ok(rendered) => format!("`{label}` completed\n\n{rendered}"),
                Err(err) => format!("`{label}` failed to launch: {err}\n"),
            };
            let _ = tui_tx.send(TuiEvent::TextDelta(rendered));
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

async fn run_archon(args: Vec<String>) -> Result<String> {
    let exe = std::env::current_exe()?;
    let output = tokio::process::Command::new(exe)
        .args(args)
        .output()
        .await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut rendered = String::new();
    if !stdout.trim().is_empty() {
        rendered.push_str(stdout.trim_end());
        rendered.push('\n');
    }
    if !stderr.trim().is_empty() {
        rendered.push_str("\nstderr:\n");
        rendered.push_str(stderr.trim_end());
        rendered.push('\n');
    }
    if rendered.trim().is_empty() {
        rendered.push_str(&format!("exit status: {}\n", output.status));
    } else if !output.status.success() {
        rendered.push_str(&format!("\nexit status: {}\n", output.status));
    }
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::registry::default_registry;

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
            "auth",
            "chat",
        ] {
            assert!(
                registry.is_primary(primary),
                "/{primary} must be registered"
            );
        }
    }
}
