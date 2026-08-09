//! `archon ide-stdio` — JSON-RPC over stdin/stdout for IDE extensions.
//!
//! Boots the same session agent the print and headless modes use, attaches it
//! to the IDE protocol handler, and lets the handler stream the turn back as
//! `archon/textDelta` / `archon/turnComplete` notifications.
//!
//! Tools are live here. They are safe to enable because
//! `IdeAgentRuntime::new` installs the agent's permission channel on the way
//! in, so `request_tool_permission` blocks for a real answer from the editor
//! instead of auto-approving — see `crates/archon-sdk/src/ide/runtime.rs`.

use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use archon_core::cli_flags::ResolvedFlags;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_sdk::ide::handler::IdeProtocolHandler;
use archon_sdk::ide::stdio::StdioTransport;

use crate::cli_args::Cli;
use crate::session::BuiltAgent;
use crate::session::build_agent::build_session_agent;

/// How long to wait for the sandbox audit writer to flush on shutdown.
/// Matches the interactive and headless session paths.
const AUDIT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn handle_ide_stdio_command(
    workspace: Option<PathBuf>,
    cli: &Cli,
    config: &ArchonConfig,
    env_vars: &ArchonEnvVars,
    resolved_flags: &ResolvedFlags,
) -> anyhow::Result<()> {
    if let Some(root) = workspace.as_deref() {
        enter_workspace(root)?;
    }

    let session_id = uuid::Uuid::new_v4().to_string();
    let BuiltAgent {
        agent,
        event_rx,
        sandbox_audit_drain,
        permission_mode,
        ..
    } = build_session_agent(config, &session_id, cli, env_vars, resolved_flags, false)
        .await
        .map_err(|exit_code| {
            anyhow::anyhow!("IDE session bootstrap failed (exit code {exit_code})")
        })?;

    let effective_mode = adopt_interactive_permission_mode(&permission_mode).await;
    tracing::info!(%session_id, mode = %effective_mode, "IDE stdio mode: agent attached with tools");

    let (handler, notifications, _agent) =
        IdeProtocolHandler::with_agent(env!("CARGO_PKG_VERSION"), agent, event_rx);
    let mut transport = StdioTransport::new(handler);
    let loop_result = transport.run_with_events(notifications).await;
    let audit_result = sandbox_audit_drain.shutdown(AUDIT_DRAIN_TIMEOUT).await;

    match (loop_result, audit_result) {
        (Ok(()), Ok(readback)) => {
            tracing::info!(
                accepted = readback.accepted,
                dropped = readback.dropped,
                persisted = readback.persisted,
                failed = readback.failed,
                "sandbox audit writer drained"
            );
            Ok(())
        }
        // A transport failure is the one the IDE can act on, so it wins; the
        // audit failure is still surfaced rather than swallowed.
        (Err(loop_error), Ok(_)) => Err(loop_error),
        (Ok(()), Err(audit_error)) => Err(audit_error),
        (Err(loop_error), Err(audit_error)) => Err(anyhow::anyhow!(
            "IDE stdio loop failed: {loop_error:#}; sandbox audit drain failed: {audit_error:#}"
        )),
    }
}

/// Move the process into the project root the IDE named.
///
/// The agent's working directory, its per-project stores under `.archon/`,
/// and the project context in the system prompt all derive from the process
/// cwd, and an IDE is free to spawn a helper process anywhere. Project
/// configuration has already been resolved from the spawn cwd by the time we
/// get here, so this relocates the session, not the configuration — which is
/// why the extension is expected to spawn us in the workspace as well.
fn enter_workspace(root: &Path) -> anyhow::Result<()> {
    std::env::set_current_dir(root)
        .map_err(|error| anyhow::anyhow!("--workspace {}: {error}", root.display()))?;
    tracing::info!(workspace = %root.display(), "IDE stdio mode: workspace set");
    Ok(())
}

/// Upgrade an `auto` session to `default` for an IDE run, and report the mode
/// the session will actually use.
///
/// `auto` means "decide without me": on a `NeedsPermission` verdict the agent
/// consults its [`AutoModeEvaluator`] and *denies* anything risky outright —
/// `request_tool_permission` is never called, so no `archon/permissionRequest`
/// is ever sent and the editor's approval UI can never appear. That is the
/// right behaviour for a headless run and the wrong one for a window with a
/// human in it, so an IDE session asks instead.
///
/// Only `auto` is touched. `dontAsk` and `bypassPermissions` are deliberate
/// statements that the user does not want to be asked, and honouring an
/// explicit choice matters more than showing off the prompt.
///
/// [`AutoModeEvaluator`]: archon_permissions::auto::AutoModeEvaluator
async fn adopt_interactive_permission_mode(permission_mode: &tokio::sync::Mutex<String>) -> String {
    let mut mode = permission_mode.lock().await;
    if *mode == "auto" {
        tracing::info!("IDE stdio mode: permission mode auto -> default so the editor is asked");
        "default".clone_into(&mut mode);
    }
    mode.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_auto_session_is_upgraded_so_the_editor_gets_asked() {
        let mode = tokio::sync::Mutex::new("auto".to_string());

        let effective = adopt_interactive_permission_mode(&mode).await;

        assert_eq!(effective, "default");
        assert_eq!(*mode.lock().await, "default");
    }

    #[tokio::test]
    async fn an_explicit_choice_not_to_be_asked_is_left_alone() {
        for chosen in ["dontAsk", "bypassPermissions", "plan", "acceptEdits"] {
            let mode = tokio::sync::Mutex::new(chosen.to_string());

            let effective = adopt_interactive_permission_mode(&mode).await;

            assert_eq!(effective, chosen, "{chosen} must survive an IDE session");
        }
    }
}
