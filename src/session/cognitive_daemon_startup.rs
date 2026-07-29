use std::path::Path;

use archon_tui::app::TuiEvent;
use archon_tui::event_channel::TuiEventSender;

pub(super) fn ensure_for_session(
    config: &archon_core::config::ArchonConfig,
    working_dir: &Path,
    tui_tx: &TuiEventSender,
) {
    use crate::command::cognitive_daemon::DaemonStartOutcome;

    match crate::command::cognitive_daemon::ensure_daemon_started(config, working_dir) {
        Ok(DaemonStartOutcome::Disabled) => {}
        Ok(DaemonStartOutcome::PolicyDenied(reason)) => {
            if let Err(error) = tui_tx.send(TuiEvent::Error(format!(
                "Cognitive daemon is enabled but policy blocked startup: {reason}"
            ))) {
                tracing::warn!(%error, "cognitive daemon policy notification delivery failed");
            }
        }
        Ok(DaemonStartOutcome::AlreadyRunning { state_path }) => {
            tracing::debug!(
                state = %state_path.display(),
                "cognitive daemon already running for session"
            );
        }
        Ok(DaemonStartOutcome::Started { pid, state_path }) => {
            if let Err(error) = tui_tx.send(TuiEvent::TextDelta(format!(
                "\nCognitive daemon started (pid {pid}).\nState: {}\n",
                state_path.display()
            ))) {
                tracing::warn!(%error, "cognitive daemon startup notification delivery failed");
            }
        }
        Err(error) => {
            if let Err(delivery_error) = tui_tx.send(TuiEvent::Error(format!(
                "Cognitive daemon auto-start failed: {error}"
            ))) {
                tracing::warn!(
                    %delivery_error,
                    "cognitive daemon failure notification delivery failed"
                );
            }
        }
    }
}
