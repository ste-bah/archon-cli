struct ExecutionResult {
    result: Result<crate::subagent_executor::SubagentCompletion, ExecutorError>,
    cancelled: bool,
}

impl ExecutionResult {
    fn outcome(&self) -> SubagentOutcome {
        if self.cancelled {
            SubagentOutcome::Cancelled
        } else {
            match &self.result {
                Ok(completion) => SubagentOutcome::Completed(completion.clone()),
                Err(ExecutorError::ProviderFailure {
                    message,
                    status,
                    retry_after_secs,
                    overloaded,
                    source,
                }) => SubagentOutcome::FailedWithProvider {
                    message: message.clone(),
                    status: *status,
                    retry_after_secs: *retry_after_secs,
                    overloaded: *overloaded,
                    source: source.clone(),
                },
                Err(ExecutorError::ControlPaused(message)) => {
                    SubagentOutcome::ControlPaused(message.clone())
                }
                Err(ExecutorError::InvalidAnswer(message)) => {
                    SubagentOutcome::InvalidAnswer(message.clone())
                }
                Err(error) => SubagentOutcome::Failed(error.to_string()),
            }
        }
    }

    fn terminal_status(&self) -> AgentStatus {
        if self.cancelled || matches!(self.result, Err(ExecutorError::ControlPaused(_))) {
            AgentStatus::Cancelled
        } else if self.result.is_ok() {
            AgentStatus::Finished
        } else {
            AgentStatus::Failed
        }
    }
}
