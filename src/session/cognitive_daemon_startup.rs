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
            let _ = tui_tx.send(TuiEvent::Error(format!(
                "Cognitive daemon is enabled but policy blocked startup: {reason}"
            )));
        }
        Ok(DaemonStartOutcome::AlreadyRunning { state_path }) => {
            tracing::debug!(
                state = %state_path.display(),
                "cognitive daemon already running for session"
            );
        }
        Ok(DaemonStartOutcome::Started { pid, state_path }) => {
            let _ = tui_tx.send(TuiEvent::TextDelta(format!(
                "\nCognitive daemon started (pid {pid}).\nState: {}\n",
                state_path.display()
            )));
        }
        Err(error) => {
            let _ = tui_tx.send(TuiEvent::Error(format!(
                "Cognitive daemon auto-start failed: {error}"
            )));
        }
    }
}
