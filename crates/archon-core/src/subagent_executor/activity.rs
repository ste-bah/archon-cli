use super::*;

impl AgentSubagentExecutor {
    pub(super) fn emit_subagent_started(&self, subagent_id: &str, agent_type: &str, model: &str) {
        self.emit_subagent_activity(
            subagent_id,
            agent_type,
            model,
            archon_observability::AgentActivityKind::AgentSpawned,
            archon_observability::AgentActivityStatus::Running,
            format!("{agent_type} running"),
        );
    }

    pub(super) fn emit_subagent_finished(
        &self,
        subagent_id: &str,
        agent_type: &str,
        model: &str,
        result: &Result<String, String>,
    ) {
        match result {
            Ok(_) => self.emit_subagent_activity(
                subagent_id,
                agent_type,
                model,
                archon_observability::AgentActivityKind::AgentCompleted,
                archon_observability::AgentActivityStatus::Completed,
                format!("{agent_type} completed"),
            ),
            Err(err) => self.emit_subagent_activity(
                subagent_id,
                agent_type,
                model,
                archon_observability::AgentActivityKind::AgentFailed,
                archon_observability::AgentActivityStatus::Failed,
                err.clone(),
            ),
        }
    }

    fn emit_subagent_activity(
        &self,
        subagent_id: &str,
        agent_type: &str,
        model: &str,
        kind: archon_observability::AgentActivityKind,
        status: archon_observability::AgentActivityStatus,
        message: impl Into<String>,
    ) {
        let Some(sink) = &self.agent_config.activity_sink else {
            return;
        };
        sink.emit(
            archon_observability::AgentActivityEvent::new(
                self.session_id.clone(),
                kind,
                status,
                message,
            )
            .with_subagent_id(subagent_id.to_string())
            .with_agent_key(agent_type.to_string())
            .with_subagent_type(agent_type.to_string())
            .with_provider_model(self.client.name().to_string(), model.to_string()),
        );
    }
}
