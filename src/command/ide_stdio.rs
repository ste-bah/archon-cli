//! `archon ide-stdio` — JSON-RPC over stdin/stdout for IDE extensions.
//!
//! Boots the same session agent the print and headless modes use, attaches it
//! to the IDE protocol handler, and lets the handler stream the turn back as
//! `archon/textDelta` / `archon/turnComplete` notifications.
//!
//! Scope (issue #26, first slice): read-only chat. Tool execution and the IDE
//! permission round-trip are later slices, and the agent is deliberately built
//! without tools until they land — see `ide_session_flags`.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use archon_core::cli_flags::ResolvedFlags;
use archon_core::config::ArchonConfig;
use archon_core::env_vars::ArchonEnvVars;
use archon_sdk::ide::handler::IdeProtocolHandler;
use archon_sdk::ide::stdio::StdioTransport;
use tokio::sync::Mutex;

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
        ..
    } = build_session_agent(
        config,
        &session_id,
        cli,
        env_vars,
        &ide_session_flags(resolved_flags),
        false,
    )
    .await
    .map_err(|exit_code| anyhow::anyhow!("IDE session bootstrap failed (exit code {exit_code})"))?;

    tracing::info!(%session_id, "IDE stdio mode: agent attached (text-only slice)");

    let (handler, notifications) = IdeProtocolHandler::with_agent(
        env!("CARGO_PKG_VERSION"),
        Arc::new(Mutex::new(agent)),
        event_rx,
    );
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

/// Narrow the session's resolved flags for an IDE stdio run.
///
/// The empty whitelist is a safety requirement, not a preference. [`Agent`]
/// auto-approves every permission request when its `permission_response_rx`
/// is `None` (`archon-core/src/agent/permission_gate.rs`), and this slice has
/// no IDE permission UI to supply one — so a tool-capable agent here would run
/// Bash and Write without anybody being asked. An empty whitelist strips the
/// registry *and* the tool schemas advertised to the model, so the model has
/// nothing to call. Lift this only together with the permission round-trip.
///
/// [`Agent`]: archon_core::agent::Agent
fn ide_session_flags(resolved_flags: &ResolvedFlags) -> ResolvedFlags {
    let mut flags = resolved_flags.clone();
    flags.tool_whitelist = Some(Vec::new());
    flags
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ide_sessions_advertise_no_tools() {
        let flags = ide_session_flags(&ResolvedFlags::default());

        assert_eq!(
            flags.tool_whitelist,
            Some(Vec::new()),
            "the IDE slice must not hand the model a tool it can auto-approve"
        );
    }

    #[test]
    fn narrowing_tools_leaves_the_rest_of_the_flags_alone() {
        let resolved = ResolvedFlags {
            model: Some("some-model".to_string()),
            verbose: true,
            ..ResolvedFlags::default()
        };

        let flags = ide_session_flags(&resolved);

        assert_eq!(flags.model.as_deref(), Some("some-model"));
        assert!(flags.verbose);
    }
}
