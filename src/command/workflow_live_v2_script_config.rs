use super::WorkflowV2ScriptRunner;
use std::sync::Arc;

impl WorkflowV2ScriptRunner {
    pub(crate) fn with_resolved_config(
        mut self,
        config: Arc<archon_core::config::ArchonConfig>,
    ) -> Self {
        self.resolved_config = Some(config);
        self
    }

    #[cfg(test)]
    pub(in crate::command::workflow_live) fn tool_host_for_test(
        &self,
    ) -> archon_workflow::WorkflowResult<
        crate::command::workflow_live::workflow_script_tools::ScriptToolHost,
    > {
        super::workflow_live_v2_script_host::WorkflowScriptHost::tool_host_for_test(self)
    }

    pub(crate) fn with_host_command_executor(
        mut self,
        executor: Arc<dyn crate::command::workflow_host_command_exec::WorkflowHostCommandExecutor>,
    ) -> Self {
        self.host_command_executor = Some(executor);
        self
    }
}
