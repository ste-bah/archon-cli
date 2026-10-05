use super::*;

use archon_observability::{AgentActivityKind as Kind, AgentActivityStatus as Status};

/// The activity row one call opens, and the duty to close it (Issue 322).
///
/// A row opens at the first non-terminal event of a call: queued, when it
/// waits for a slot, or started. Readers such as the TUI remove a subagent
/// row only on a terminal event, so every way a call ends after its row opens
/// must send one. The run's own completion sends it through
/// [`ActivityRow::finished`]; an error before that (a cancel while queued, a
/// closed slot semaphore, a refused registration or preparation) through
/// [`ActivityRow::settle`]; a call whose future is dropped part-way, through
/// `Drop`. A row that is not open sends nothing, so no path sends a second
/// terminal event. Every sink's `emit` is synchronous and never blocks on a
/// reader, so the drop can send.
pub(super) struct ActivityRow {
    sink: Option<Arc<dyn archon_observability::AgentActivitySink>>,
    session_id: String,
    provider: String,
    subagent_id: String,
    /// The agent type and model of the open row; `None` while none is open.
    open: Option<(String, String)>,
}

impl ActivityRow {
    pub(super) fn queued(&mut self, agent_type: &str, model: &str, message: String) {
        self.open_with(
            agent_type,
            model,
            Kind::AgentQueued,
            Status::Queued,
            message,
        );
    }

    pub(super) fn slot_acquired(&mut self, agent_type: &str, model: &str, message: String) {
        self.open_with(
            agent_type,
            model,
            Kind::AgentRunning,
            Status::Running,
            message,
        );
    }

    pub(super) fn started(&mut self, agent_type: &str, model: &str) {
        self.open_with(
            agent_type,
            model,
            Kind::AgentSpawned,
            Status::Running,
            format!("{agent_type} running"),
        );
    }

    /// The run's own terminal event; the row is closed after it.
    pub(super) fn finished(
        &mut self,
        agent_type: &str,
        model: &str,
        result: &Result<String, String>,
    ) {
        self.open = None;
        let (kind, status, message) = match result {
            Ok(_) => (
                Kind::AgentCompleted,
                Status::Completed,
                format!("{agent_type} completed"),
            ),
            Err(err) => (Kind::AgentFailed, Status::Failed, err.clone()),
        };
        self.emit(agent_type, model, kind, status, message);
    }

    /// Close a row the run left open with the call's `result`: cancelled
    /// when `cancel` fired, failed on any other error.
    pub(super) fn settle(
        &mut self,
        result: &Result<String, ExecutorError>,
        cancel: &CancellationToken,
    ) {
        let Some((agent_type, _)) = &self.open else {
            return;
        };
        match result {
            Ok(_) => {
                let message = format!("{agent_type} completed");
                self.close(Kind::AgentCompleted, Status::Completed, message);
            }
            Err(err) if cancel.is_cancelled() => {
                self.close(Kind::Cancelled, Status::Cancelled, err.to_string())
            }
            Err(err) => self.close(Kind::AgentFailed, Status::Failed, err.to_string()),
        }
    }

    fn open_with(
        &mut self,
        agent_type: &str,
        model: &str,
        kind: Kind,
        status: Status,
        message: String,
    ) {
        self.open = Some((agent_type.to_string(), model.to_string()));
        self.emit(agent_type, model, kind, status, message);
    }

    fn close(&mut self, kind: Kind, status: Status, message: String) {
        if let Some((agent_type, model)) = self.open.take() {
            self.emit(&agent_type, &model, kind, status, message);
        }
    }

    fn emit(&self, agent_type: &str, model: &str, kind: Kind, status: Status, message: String) {
        let Some(sink) = &self.sink else {
            return;
        };
        sink.emit(
            archon_observability::AgentActivityEvent::new(
                self.session_id.clone(),
                kind,
                status,
                message,
            )
            .with_subagent_id(self.subagent_id.clone())
            .with_agent_key(agent_type.to_string())
            .with_subagent_type(agent_type.to_string())
            .with_provider_model(self.provider.clone(), model.to_string()),
        );
    }
}

impl Drop for ActivityRow {
    fn drop(&mut self) {
        if let Some((agent_type, _)) = &self.open {
            let message = format!("{agent_type} stopped: its call was dropped before it finished");
            self.close(Kind::Cancelled, Status::Cancelled, message);
        }
    }
}

impl AgentSubagentExecutor {
    /// The activity row of call `subagent_id`, not yet open.
    pub(super) fn activity_row(&self, subagent_id: &str) -> ActivityRow {
        ActivityRow {
            sink: self.agent_config.activity_sink.clone(),
            session_id: self.session_id.clone(),
            provider: self.client.name().to_string(),
            subagent_id: subagent_id.to_string(),
            open: None,
        }
    }
}
